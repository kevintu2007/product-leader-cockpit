// A command taking a path from the webview (ADR 0011).
#[tauri::command]
pub fn open_anything(file_path: String) -> Result<(), String> {
    Ok(())
}
