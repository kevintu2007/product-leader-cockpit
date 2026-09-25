fn install_plugin() {
    tauri::Builder::default().plugin(tauri_plugin_fs::init());
}
