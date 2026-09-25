fn spawn_changed_executable() {
    let _child = std::process::Command::new("different-executable");
}
