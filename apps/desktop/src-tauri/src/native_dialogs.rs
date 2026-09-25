//! The host's native dialogs (ADR 0011). The only module that may call `rfd`;
//! the policy check pins the reviewed block below by hash, so any change to it
//! is reviewed. A chosen path stays in the host: callers validate it and
//! either store it or hold it behind an opaque token, and never return it
//! over IPC.

use std::path::PathBuf;
use std::sync::mpsc;

/// Ask the person for a folder, as a modal dialog owned by `parent`. `None`
/// when they cancel or the dialog could not be shown.
///
/// The dialog is dispatched explicitly onto the main (UI) thread, where a
/// window-parented modal dialog belongs; the caller's async task waits for
/// the answer without blocking that thread. Must not be awaited while holding
/// the Ledger or settings lock.
#[cfg(windows)]
pub async fn pick_folder(parent: tauri::Window, title: String) -> Option<PathBuf> {
    let (answer, receive) = mpsc::channel::<Option<PathBuf>>();
    let owner = parent.clone();
    parent
        .run_on_main_thread(move || {
            // PMC-REVIEWED-FOLDER-PICKER-HARNESS-START
            let chosen = rfd::FileDialog::new()
                .set_parent(&owner)
                .set_title(&title)
                .pick_folder();
            // PMC-REVIEWED-FOLDER-PICKER-HARNESS-END
            let _ = answer.send(chosen);
        })
        .ok()?;
    tauri::async_runtime::spawn_blocking(move || receive.recv().ok().flatten())
        .await
        .ok()
        .flatten()
}

/// Other platforms have no reviewed dialog yet (ADR 0003: Windows first).
#[cfg(not(windows))]
pub async fn pick_folder(_parent: tauri::Window, _title: String) -> Option<PathBuf> {
    None
}

/// Ask the person for an Operational Backup file (DG3 restore amendment
/// §3.1), filtered to the archive extension, as a modal dialog owned by
/// `parent`. Same threading and lock rules as [`pick_folder`].
#[cfg(windows)]
pub async fn pick_backup_file(
    parent: tauri::Window,
    title: String,
    filter_name: String,
) -> Option<PathBuf> {
    let (answer, receive) = mpsc::channel::<Option<PathBuf>>();
    let owner = parent.clone();
    parent
        .run_on_main_thread(move || {
            // PMC-REVIEWED-FILE-PICKER-HARNESS-START
            let chosen = rfd::FileDialog::new()
                .set_parent(&owner)
                .set_title(&title)
                .add_filter(&filter_name, &["age"])
                .pick_file();
            // PMC-REVIEWED-FILE-PICKER-HARNESS-END
            let _ = answer.send(chosen);
        })
        .ok()?;
    tauri::async_runtime::spawn_blocking(move || receive.recv().ok().flatten())
        .await
        .ok()
        .flatten()
}

#[cfg(not(windows))]
pub async fn pick_backup_file(
    _parent: tauri::Window,
    _title: String,
    _filter_name: String,
) -> Option<PathBuf> {
    None
}

/// Ask the person for an Evidence file (DG3 Vault-root and Evidence-from-file
/// amendment §4.1), opened in the Vault folder `start`, as a modal dialog
/// owned by `parent`. The start folder is a convenience, not a boundary: the
/// caller checks the answer against the Vault. Same threading and lock rules
/// as [`pick_folder`].
#[cfg(windows)]
pub async fn pick_evidence_file(
    parent: tauri::Window,
    title: String,
    start: PathBuf,
) -> Option<PathBuf> {
    let (answer, receive) = mpsc::channel::<Option<PathBuf>>();
    let owner = parent.clone();
    parent
        .run_on_main_thread(move || {
            // PMC-REVIEWED-EVIDENCE-FILE-PICKER-HARNESS-START
            let chosen = rfd::FileDialog::new()
                .set_parent(&owner)
                .set_title(&title)
                .set_directory(&start)
                .pick_file();
            // PMC-REVIEWED-EVIDENCE-FILE-PICKER-HARNESS-END
            let _ = answer.send(chosen);
        })
        .ok()?;
    tauri::async_runtime::spawn_blocking(move || receive.recv().ok().flatten())
        .await
        .ok()
        .flatten()
}

#[cfg(not(windows))]
pub async fn pick_evidence_file(
    _parent: tauri::Window,
    _title: String,
    _start: PathBuf,
) -> Option<PathBuf> {
    None
}
/// Tell the person another PMC is already running (item ⑩), as a plain
/// message box with one button. Shown before any window exists, so it has no
/// parent and blocks the calling thread until dismissed; nothing is held.
#[cfg(windows)]
pub fn show_already_running(title: &str, body: &str) {
    // PMC-REVIEWED-MESSAGE-BOX-HARNESS-START
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Info)
        .set_title(title)
        .set_description(body)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
    // PMC-REVIEWED-MESSAGE-BOX-HARNESS-END
}

#[cfg(not(windows))]
pub fn show_already_running(_title: &str, _body: &str) {}
