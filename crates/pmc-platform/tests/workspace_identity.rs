use std::fs;
#[cfg(windows)]
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_platform::settings::ProtectedSettingsRoot;
use pmc_platform::workspace::{WorkspaceError, WorkspaceIdentity, WorkspaceKind};

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// See the identical helper in `settings_store.rs` for the full rationale:
/// on this dev machine, the test process runs inside a Windows app-package
/// container that transparently redirects writes under the real
/// `%LOCALAPPDATA%`, which trips `ProtectedSettingsRoot::prepare`'s
/// `canonical == path` check for a benign OS reason unrelated to what that
/// check defends against. Each `tests/*.rs` file compiles to its own
/// process, so this override has to be repeated here rather than shared.
#[cfg(windows)]
fn ensure_test_app_data_root_is_not_redirected() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("pmc-platform-test-app-data");
        std::fs::create_dir_all(&root)
            .unwrap_or_else(|error| panic!("test app-data override root setup failed: {error}"));
        let root = std::fs::canonicalize(&root)
            .unwrap_or_else(|error| panic!("test app-data override root setup failed: {error}"));
        // SAFETY: this runs once, guarded by `Once`, before any test spawns
        // additional threads or reads `LOCALAPPDATA`; `Once::call_once`
        // establishes a happens-before edge for every later caller.
        unsafe {
            std::env::set_var("LOCALAPPDATA", root);
        }
    });
}

#[cfg(not(windows))]
fn ensure_test_app_data_root_is_not_redirected() {}

fn isolated_root(test_name: &str) -> ProtectedSettingsRoot {
    ensure_test_app_data_root_is_not_redirected();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = format!("product-mission-control-workspace-{test_name}-{nonce}-{sequence}");
    ProtectedSettingsRoot::prepare(&name)
        .unwrap_or_else(|error| panic!("protected root setup failed: {error}"))
}

#[test]
fn training_and_live_roots_are_distinct_non_overlapping_children() {
    let root = isolated_root("identity");
    let training = WorkspaceIdentity::resolve(&root, WorkspaceKind::Training)
        .unwrap_or_else(|error| panic!("training resolution failed: {error}"));
    let live = WorkspaceIdentity::resolve(&root, WorkspaceKind::Live)
        .unwrap_or_else(|error| panic!("live resolution failed: {error}"));

    assert_ne!(training.root().as_path(), live.root().as_path());
    assert!(training.root().as_path().starts_with(root.path()));
    assert!(live.root().as_path().starts_with(root.path()));
    assert!(!training.root().as_path().starts_with(live.root().as_path()));
    assert!(!live.root().as_path().starts_with(training.root().as_path()));
    assert!(training.root().as_path().is_absolute());
    assert!(live.root().as_path().is_absolute());

    fs::remove_dir_all(root.path()).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn resolving_identity_does_not_create_workspace_directories() {
    let root = isolated_root("no-side-effects");
    let training = WorkspaceIdentity::resolve(&root, WorkspaceKind::Training)
        .unwrap_or_else(|error| panic!("training resolution failed: {error}"));

    assert!(!training.root().as_path().exists());
    assert!(!root.path().join("workspaces").exists());
    fs::remove_dir_all(root.path()).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn existing_link_or_non_directory_cannot_alias_a_workspace() {
    let root = isolated_root("path-safety");
    let workspaces = root.path().join("workspaces");
    fs::create_dir_all(&workspaces)
        .unwrap_or_else(|error| panic!("workspace parent setup failed: {error}"));
    let occupied = workspaces.join("training");
    fs::write(&occupied, b"synthetic marker")
        .unwrap_or_else(|error| panic!("synthetic marker setup failed: {error}"));

    assert_eq!(
        WorkspaceIdentity::resolve(&root, WorkspaceKind::Training),
        Err(WorkspaceError::WorkspacePathOccupied)
    );

    fs::remove_dir_all(root.path()).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[cfg(any(windows, unix))]
#[test]
fn descendant_link_cannot_alias_a_workspace_when_links_are_supported() {
    let root = isolated_root("descendant-link");
    let workspaces = root.path().join("workspaces");
    let external = root.path().join("synthetic-external");
    fs::create_dir_all(&workspaces)
        .unwrap_or_else(|error| panic!("workspace parent setup failed: {error}"));
    fs::create_dir_all(&external).unwrap_or_else(|error| panic!("external setup failed: {error}"));
    let training = workspaces.join("training");
    create_directory_link(&external, &training)
        .unwrap_or_else(|error| panic!("directory link setup failed: {error}"));

    assert_eq!(
        WorkspaceIdentity::resolve(&root, WorkspaceKind::Training),
        Err(WorkspaceError::WorkspacePathContainsLink)
    );

    remove_directory_link(&training).unwrap_or_else(|error| panic!("link cleanup failed: {error}"));
    fs::remove_dir_all(root.path()).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn synthetic_seed_is_allowed_only_for_training() {
    let root = isolated_root("seed-policy");
    let second_root = isolated_root("seed-policy-second-root");
    let training = WorkspaceIdentity::resolve(&root, WorkspaceKind::Training)
        .unwrap_or_else(|error| panic!("training resolution failed: {error}"));
    let second_training = WorkspaceIdentity::resolve(&second_root, WorkspaceKind::Training)
        .unwrap_or_else(|error| panic!("second training resolution failed: {error}"));
    let live = WorkspaceIdentity::resolve(&root, WorkspaceKind::Live)
        .unwrap_or_else(|error| panic!("live resolution failed: {error}"));

    let training_permit = training
        .synthetic_seed_policy()
        .authorize()
        .unwrap_or_else(|error| panic!("training authorization failed: {error}"));
    assert_eq!(
        training.accept_synthetic_seed_permit(training_permit.clone()),
        Ok(())
    );
    assert_eq!(
        live.synthetic_seed_policy().authorize(),
        Err(WorkspaceError::LiveSyntheticSeedDenied)
    );
    assert_eq!(
        live.accept_synthetic_seed_permit(training_permit),
        Err(WorkspaceError::SyntheticSeedPermitMismatch)
    );
    let cross_root_permit = training
        .synthetic_seed_policy()
        .authorize()
        .unwrap_or_else(|error| panic!("training authorization failed: {error}"));
    assert_eq!(
        second_training.accept_synthetic_seed_permit(cross_root_permit),
        Err(WorkspaceError::SyntheticSeedPermitMismatch)
    );
    assert!(!root.path().join("workspaces").exists());

    fs::remove_dir_all(root.path()).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
    fs::remove_dir_all(second_root.path())
        .unwrap_or_else(|error| panic!("second root cleanup failed: {error}"));
}

#[cfg(any(windows, unix))]
#[test]
fn replacing_protected_root_with_a_link_fails_closed_when_links_are_supported() {
    let root = isolated_root("root-replacement");
    let original = root.path().to_path_buf();
    let external = original.with_file_name(format!(
        "{}-external",
        original
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("synthetic-workspace")
    ));
    fs::create_dir_all(&external)
        .unwrap_or_else(|error| panic!("external synthetic root setup failed: {error}"));
    fs::remove_dir_all(&original)
        .unwrap_or_else(|error| panic!("root replacement setup failed: {error}"));

    create_directory_link(&external, &original)
        .unwrap_or_else(|error| panic!("root replacement link setup failed: {error}"));

    assert_eq!(
        WorkspaceIdentity::resolve(&root, WorkspaceKind::Training),
        Err(WorkspaceError::InvalidProtectedRoot)
    );

    remove_directory_link(&original).unwrap_or_else(|error| panic!("link cleanup failed: {error}"));
    fs::remove_dir_all(&external)
        .unwrap_or_else(|error| panic!("external cleanup failed: {error}"));
}

#[cfg(any(windows, unix))]
fn create_directory_link(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    create_directory_link_impl(target, link)
}

#[cfg(windows)]
fn create_directory_link_impl(
    target: &std::path::Path,
    link: &std::path::Path,
) -> std::io::Result<()> {
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
fn create_directory_link_impl(
    target: &std::path::Path,
    link: &std::path::Path,
) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// Removes a directory link created by `create_directory_link`. Windows'
/// `RemoveDirectoryW` (what `fs::remove_dir` calls) accepts a directory
/// junction as a directory entry, but POSIX `rmdir()` (what `fs::remove_dir`
/// calls on Unix) requires the path to literally be a directory, not a
/// symlink to one -- it fails closed with `ENOTDIR` ("Not a directory") on a
/// symlink regardless of what it points to. `create_directory_link` makes a
/// real `symlink()` on Unix, so its cleanup must use `fs::remove_file`
/// (`unlink()`), matching what actually removes a symlink on that platform.
#[cfg(any(windows, unix))]
fn remove_directory_link(link: &std::path::Path) -> std::io::Result<()> {
    remove_directory_link_impl(link)
}

#[cfg(windows)]
fn remove_directory_link_impl(link: &std::path::Path) -> std::io::Result<()> {
    fs::remove_dir(link)
}

#[cfg(unix)]
fn remove_directory_link_impl(link: &std::path::Path) -> std::io::Result<()> {
    fs::remove_file(link)
}

/// The synthetic Vault root is derived only for Training, and only from the
/// already-validated workspace root. Live returns `None` rather than a
/// convention path, because the Product Vault design makes the Live Vault root
/// user-selected and no configuration surface exists yet.
#[test]
fn only_training_derives_a_synthetic_vault_root_and_deriving_it_touches_no_disk() {
    let root = isolated_root("synthetic-vault-root");
    let training = WorkspaceIdentity::resolve(&root, WorkspaceKind::Training)
        .unwrap_or_else(|error| panic!("training resolution failed: {error}"));
    let live = WorkspaceIdentity::resolve(&root, WorkspaceKind::Live)
        .unwrap_or_else(|error| panic!("live resolution failed: {error}"));

    let vault = training
        .synthetic_vault_root()
        .unwrap_or_else(|| panic!("training must derive a synthetic Vault root"));
    assert_eq!(vault.parent(), Some(training.root().as_path()));
    assert_eq!(live.synthetic_vault_root(), None);

    // Deriving is pure: neither the workspaces tree nor the Vault appears.
    assert!(!root.path().join("workspaces").exists());
    assert!(!vault.exists());

    fs::remove_dir_all(root.path()).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}
