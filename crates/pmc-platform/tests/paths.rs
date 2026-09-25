use std::fs;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_platform::paths::{resolve_contained_path, validate_canonical_root, PathValidationError};

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn isolated_dir(test_name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "pmc-synthetic-paths-{test_name}-{nonce}-{sequence}"
    ));
    fs::create_dir_all(&path).unwrap_or_else(|error| panic!("fixture setup failed: {error}"));
    fs::canonicalize(&path).unwrap_or_else(|error| panic!("fixture canonicalize failed: {error}"))
}

#[test]
fn an_absolute_canonical_directory_validates_successfully() {
    let root = isolated_dir("valid-root");
    let validated = validate_canonical_root(&root).unwrap();
    assert_eq!(validated, root);
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn a_relative_root_is_rejected() {
    let result = validate_canonical_root(Path::new("relative/vault"));
    assert_eq!(result, Err(PathValidationError::RootNotAbsolute));
}

#[test]
fn a_root_containing_a_parent_dir_component_is_rejected() {
    // `isolated_dir` canonicalizes to a Windows verbatim (`\\?\`) path, which
    // is never textually normalized -- a literal ".." there is just an
    // (invalid) directory name, not a parent-dir traversal. Real-world
    // caller-supplied roots are ordinary drive-letter paths, so build this
    // fixture from `std::env::temp_dir()` directly, without canonicalizing.
    let with_dotdot = std::env::temp_dir()
        .join("pmc-synthetic-paths-dotdot-marker")
        .join("..");
    let result = validate_canonical_root(&with_dotdot);
    assert_eq!(result, Err(PathValidationError::RootNotAbsolute));
}

#[test]
fn a_root_that_is_a_file_not_a_directory_is_rejected() {
    let root = isolated_dir("file-root-parent");
    let file_path = root.join("not-a-directory");
    fs::write(&file_path, b"synthetic marker")
        .unwrap_or_else(|error| panic!("fixture setup failed: {error}"));
    let result = validate_canonical_root(&file_path);
    assert_eq!(result, Err(PathValidationError::RootIsNotADirectory));
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn a_missing_root_is_rejected() {
    let root = isolated_dir("missing-root-parent");
    let missing = root.join("does-not-exist");
    let result = validate_canonical_root(&missing);
    assert_eq!(result, Err(PathValidationError::RootUnavailable));
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn resolve_contained_path_returns_the_resolved_path_for_a_missing_file() {
    let root = isolated_dir("missing-file-root");
    let resolved =
        resolve_contained_path(&root, Path::new("Research/Competitive/notes.md")).unwrap();
    assert_eq!(
        resolved,
        root.join("Research").join("Competitive").join("notes.md")
    );
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn resolve_contained_path_accepts_a_real_nested_existing_file() {
    let root = isolated_dir("real-file-root");
    fs::create_dir_all(root.join("Research").join("Competitive"))
        .unwrap_or_else(|error| panic!("fixture setup failed: {error}"));
    let target = root.join("Research").join("Competitive").join("notes.md");
    fs::write(&target, b"synthetic evidence content")
        .unwrap_or_else(|error| panic!("fixture setup failed: {error}"));

    let resolved =
        resolve_contained_path(&root, Path::new("Research/Competitive/notes.md")).unwrap();
    assert_eq!(resolved, target);
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn resolve_contained_path_rejects_a_relative_path_escaping_via_dotdot() {
    let root = isolated_dir("escape-root");
    let result = resolve_contained_path(&root, Path::new("../secrets.md"));
    assert_eq!(result, Err(PathValidationError::PathEscapesRoot));
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn resolve_contained_path_rejects_an_absolute_relative_argument() {
    let root = isolated_dir("absolute-arg-root");
    let absolute = if cfg!(windows) {
        PathBuf::from("C:\\secrets.md")
    } else {
        PathBuf::from("/secrets.md")
    };
    let result = resolve_contained_path(&root, &absolute);
    assert_eq!(result, Err(PathValidationError::PathEscapesRoot));
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn resolve_contained_path_rejects_an_empty_relative_path() {
    let root = isolated_dir("empty-arg-root");
    let result = resolve_contained_path(&root, Path::new(""));
    assert_eq!(result, Err(PathValidationError::PathIsEmpty));
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn resolve_contained_path_rejects_a_non_directory_before_the_final_component() {
    let root = isolated_dir("occupied-root");
    fs::write(root.join("Research"), b"synthetic marker")
        .unwrap_or_else(|error| panic!("fixture setup failed: {error}"));
    let result = resolve_contained_path(&root, Path::new("Research/notes.md"));
    assert_eq!(result, Err(PathValidationError::PathOccupiedByNonDirectory));
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn resolve_contained_path_rejects_a_reparse_point_component() {
    let root = isolated_dir("reparse-root");
    let external = isolated_dir("reparse-external");
    let link = root.join("Research");
    create_directory_link(&external, &link)
        .unwrap_or_else(|error| panic!("link setup failed: {error}"));

    let result = resolve_contained_path(&root, Path::new("Research/notes.md"));
    assert_eq!(
        result,
        Err(PathValidationError::PathContainsLinkOrReparsePoint)
    );

    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
    fs::remove_dir_all(&external).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn a_root_that_is_itself_a_reparse_point_is_rejected() {
    let external = isolated_dir("root-reparse-external");
    let parent = isolated_dir("root-reparse-parent");
    let link = parent.join("vault-link");
    create_directory_link(&external, &link)
        .unwrap_or_else(|error| panic!("link setup failed: {error}"));

    let result = validate_canonical_root(&link);
    assert_eq!(result, Err(PathValidationError::RootIsLinkOrReparsePoint));

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
