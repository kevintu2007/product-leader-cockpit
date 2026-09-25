fn main() {
    println!("cargo:rerun-if-env-changed=PMC_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=PMC_BUILD_DIRTY");
    let commit =
        std::env::var("PMC_BUILD_COMMIT").unwrap_or_else(|_| "development-unbound".to_owned());
    let dirty = std::env::var("PMC_BUILD_DIRTY").unwrap_or_else(|_| "unknown".to_owned());
    println!("cargo:rustc-env=PMC_BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=PMC_BUILD_DIRTY={dirty}");
    tauri_build::build()
}
