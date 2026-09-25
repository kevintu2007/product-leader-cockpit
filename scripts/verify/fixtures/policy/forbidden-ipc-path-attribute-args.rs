// A command whose attribute has arguments, taking a path (ADR 0011).
#[tauri::command(rename_all = "snake_case")]
pub fn open_anything(target_path: String) -> Result<(), String> {
    Ok(())
}
