use std::fs;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_knowledge::vault::{observe_vault_health, VaultError, VaultHealth, VaultRoot};

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn isolated_dir(test_name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "pmc-synthetic-vault-{test_name}-{nonce}-{sequence}"
    ));
    fs::create_dir_all(&path).unwrap_or_else(|error| panic!("fixture setup failed: {error}"));
    fs::canonicalize(&path).unwrap_or_else(|error| panic!("fixture canonicalize failed: {error}"))
}

#[test]
fn a_real_directory_validates_as_a_vault_root() {
    let root = isolated_dir("valid");
    let vault = VaultRoot::validate(&root).unwrap();
    assert_eq!(vault.as_path(), root.as_path());
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn a_relative_path_is_rejected() {
    let result = VaultRoot::validate(Path::new("relative/vault"));
    assert!(matches!(result, Err(VaultError::InvalidRoot(_))));
}

#[test]
fn default_identity_name_returns_the_final_path_component() {
    let root = isolated_dir("named-vault");
    let vault = VaultRoot::validate(&root).unwrap();
    let expected = root.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(vault.default_identity_name(), Some(expected));
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn a_present_directory_observes_as_available() {
    let root = isolated_dir("available");
    let vault = VaultRoot::validate(&root).unwrap();
    assert_eq!(observe_vault_health(&vault), VaultHealth::Available);
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn a_removed_directory_observes_as_unavailable() {
    let root = isolated_dir("removed");
    let vault = VaultRoot::validate(&root).unwrap();
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
    assert_eq!(observe_vault_health(&vault), VaultHealth::Unavailable);
}

#[test]
fn a_root_replaced_by_a_reparse_point_observes_as_unavailable() {
    let external = isolated_dir("reparse-external");
    let parent = isolated_dir("reparse-parent");
    let link = parent.join("vault-link");
    fs::create_dir_all(&link).unwrap_or_else(|error| panic!("fixture setup failed: {error}"));
    let vault = VaultRoot::validate(&link).unwrap();
    fs::remove_dir_all(&link).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
    create_directory_link(&external, &link)
        .unwrap_or_else(|error| panic!("link setup failed: {error}"));

    assert_eq!(observe_vault_health(&vault), VaultHealth::Unavailable);

    fs::remove_dir_all(&parent).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
    fs::remove_dir_all(&external).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[cfg(any(windows, unix))]
fn create_directory_link(target: &Path, link: &Path) -> std::io::Result<()> {
    create_directory_link_impl(target, link)
}

#[cfg(windows)]
fn create_directory_link_impl(target: &Path, link: &Path) -> std::io::Result<()> {
    // PMC-REVIEWED-JUNCTION-HARNESS-START
    let status = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "New-Item -ItemType Junction -Path $env:PMC_JUNCTION_LINK -Target $env:PMC_JUNCTION_TARGET | Out-Null",
        ])
        .env("PMC_JUNCTION_LINK", link)
        .env("PMC_JUNCTION_TARGET", target)
        .status()?;
    // PMC-REVIEWED-JUNCTION-HARNESS-END
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other("junction helper returned failure"))
    }
}

#[cfg(unix)]
fn create_directory_link_impl(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}
