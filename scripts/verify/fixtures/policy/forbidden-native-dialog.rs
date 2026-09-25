// A dialog opened outside the reviewed block (ADR 0011).
fn pick() -> Option<std::path::PathBuf> {
    rfd::FileDialog::new().pick_file()
}
