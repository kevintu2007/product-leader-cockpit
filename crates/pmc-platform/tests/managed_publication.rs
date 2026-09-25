//! Constrained managed-projection publication primitives.

use std::fs;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_platform::managed_publication::{
    publish_managed_file, remove_managed_file, ManagedPublicationError,
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct ManagedRoot(PathBuf);

impl ManagedRoot {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock must follow the Unix epoch")
            .as_nanos();
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("pmc-managed-{nonce}-{sequence}"));
        fs::create_dir_all(&root).expect("managed root must be creatable");
        Self(fs::canonicalize(&root).expect("managed root must canonicalize"))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ManagedRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn publishing_creates_missing_directories_and_writes_exact_bytes() {
    let root = ManagedRoot::new();

    publish_managed_file(
        root.path(),
        Path::new("Products/product-1--aa.md"),
        b"---\npmc_managed: true\n---\n",
        "t0",
    )
    .expect("a contained path must publish");

    let written = fs::read(root.path().join("Products").join("product-1--aa.md")).unwrap();
    assert_eq!(written, b"---\npmc_managed: true\n---\n");
}

#[test]
fn publishing_replaces_an_existing_file_completely() {
    let root = ManagedRoot::new();
    let relative = Path::new("Products/product-1--aa.md");
    publish_managed_file(
        root.path(),
        relative,
        b"the original, much longer body",
        "t0",
    )
    .unwrap();

    publish_managed_file(root.path(), relative, b"short", "t1").unwrap();

    let written = fs::read(root.path().join("Products").join("product-1--aa.md")).unwrap();
    assert_eq!(
        written, b"short",
        "a replacement must not leave a tail of the previous, longer content"
    );
}

/// No staging artefact may survive a successful publication: a leftover
/// temp file inside the managed subtree would show up in the Vault and be
/// archived or indexed as if it were a projection.
#[test]
fn publishing_leaves_no_staging_artefact_behind() {
    let root = ManagedRoot::new();
    publish_managed_file(
        root.path(),
        Path::new("Products/product-1--aa.md"),
        b"body",
        "t0",
    )
    .unwrap();

    let names: Vec<String> = fs::read_dir(root.path().join("Products"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["product-1--aa.md".to_string()]);
}

#[test]
fn a_traversal_path_is_refused() {
    let root = ManagedRoot::new();

    assert_eq!(
        publish_managed_file(root.path(), Path::new("../escaped.md"), b"body", "t0"),
        Err(ManagedPublicationError::PathEscapesManagedRoot)
    );
}

#[test]
fn an_absolute_path_is_refused() {
    let root = ManagedRoot::new();
    let absolute = std::env::temp_dir().join("pmc-absolute-escape.md");

    assert_eq!(
        publish_managed_file(root.path(), &absolute, b"body", "t0"),
        Err(ManagedPublicationError::PathEscapesManagedRoot)
    );
    assert!(
        !absolute.exists(),
        "a refused publication must not have written anything"
    );
}

#[test]
fn removing_an_absent_file_succeeds_so_a_retry_can_finish() {
    let root = ManagedRoot::new();

    assert_eq!(
        remove_managed_file(root.path(), Path::new("Products/never-existed--zz.md")),
        Ok(())
    );
}

#[test]
fn removing_deletes_only_the_named_file() {
    let root = ManagedRoot::new();
    publish_managed_file(
        root.path(),
        Path::new("Products/product-1--aa.md"),
        b"one",
        "t0",
    )
    .unwrap();
    publish_managed_file(
        root.path(),
        Path::new("Products/product-2--bb.md"),
        b"two",
        "t1",
    )
    .unwrap();

    remove_managed_file(root.path(), Path::new("Products/product-1--aa.md")).unwrap();

    assert!(!root
        .path()
        .join("Products")
        .join("product-1--aa.md")
        .exists());
    assert!(root
        .path()
        .join("Products")
        .join("product-2--bb.md")
        .exists());
}

#[test]
fn a_traversal_removal_is_refused() {
    let root = ManagedRoot::new();

    assert_eq!(
        remove_managed_file(root.path(), Path::new("../../escaped.md")),
        Err(ManagedPublicationError::PathEscapesManagedRoot)
    );
}

#[test]
fn a_managed_root_that_is_not_a_canonical_directory_is_refused() {
    // `resolve_contained_path` only inspects the components it appends, so
    // an unvalidated root is a hole no per-path check can see: every
    // appended component is innocent while the root itself redirects the
    // whole subtree.
    let root = ManagedRoot::new();
    let file_as_root = root.path().join("not-a-directory");
    fs::write(&file_as_root, b"x").expect("fixture file must be writable");

    assert_eq!(
        publish_managed_file(&file_as_root, Path::new("Products/x.md"), b"body", "token"),
        Err(ManagedPublicationError::ManagedRootNotAnAuthorityBoundary),
    );
    assert_eq!(
        remove_managed_file(&file_as_root, Path::new("Products/x.md")),
        Err(ManagedPublicationError::ManagedRootNotAnAuthorityBoundary),
    );
}

#[test]
fn a_relative_managed_root_is_refused() {
    // A relative root resolves against whatever the process working
    // directory happens to be, which is not an authority boundary.
    assert_eq!(
        publish_managed_file(
            Path::new("managed-relative"),
            Path::new("Products/x.md"),
            b"body",
            "token"
        ),
        Err(ManagedPublicationError::ManagedRootNotAnAuthorityBoundary),
    );
}

// ---------------------------------------------------------------------------
// Stage-path TOCTOU hardening: the stage file is never opened through
// something already at its path. The parent-directory race is NOT closed by
// this; closing it needs a platform-specific handle-relative open.
// ---------------------------------------------------------------------------

fn stage_path(root: &Path, relative: &str, token: &str) -> PathBuf {
    let mut staged = root.join(relative).into_os_string();
    staged.push(".");
    staged.push(token);
    staged.push(".tmp");
    PathBuf::from(staged)
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

#[cfg(windows)]
fn create_file_link(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

#[cfg(unix)]
fn create_file_link(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(any(windows, unix))]
#[test]
fn a_link_planted_at_the_stage_path_is_refused_rather_than_followed() {
    // An attacker who can write inside the managed subtree plants a link at
    // the predictable stage name, pointing outside the root. A plain
    // `File::create` would open through it and deliver the bytes outside.
    let root = ManagedRoot::new();
    let relative = "Products/product-1--aa.md";
    fs::create_dir_all(root.path().join("Products")).unwrap();
    let outside = std::env::temp_dir().join(format!(
        "pmc-outside-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&outside).unwrap();
    let staged = stage_path(root.path(), relative, "t9");
    if create_directory_link(&outside, &staged).is_err() {
        // The harness could not create a link on this machine (for example
        // no junction privilege); there is nothing to assert about.
        let _ = fs::remove_dir_all(&outside);
        return;
    }

    let outcome = publish_managed_file(root.path(), Path::new(relative), b"payload", "t9");

    assert!(
        matches!(outcome, Err(ManagedPublicationError::PublicationFailed)),
        "publication proceeded through a planted stage link: {outcome:?}"
    );
    // Nothing landed outside the managed root, and the destination was not
    // created either.
    assert!(fs::read_dir(&outside).unwrap().next().is_none());
    assert!(!root.path().join(relative).exists());
    let _ = fs::remove_dir_all(&staged);
    let _ = fs::remove_dir_all(&outside);
}

#[test]
fn a_hard_link_planted_at_the_stage_path_does_not_reach_its_target() {
    // A hard link needs no privilege on either platform. It is a regular
    // file, so opening it with a plain `File::create` would truncate and
    // overwrite the victim it shares its data with. Unlinking the stage
    // name instead leaves the victim's bytes exactly as they were.
    let root = ManagedRoot::new();
    let relative = "Products/product-1--aa.md";
    fs::create_dir_all(root.path().join("Products")).unwrap();
    let victim = root.path().join("victim.md");
    fs::write(&victim, b"victim bytes that must survive").unwrap();
    let staged = stage_path(root.path(), relative, "t3");
    if fs::hard_link(&victim, &staged).is_err() {
        // Hard links are unavailable on this filesystem; nothing to assert.
        return;
    }

    publish_managed_file(root.path(), Path::new(relative), b"payload", "t3")
        .expect("a hard link at the stage path is a leftover name, so the retry finishes");

    assert_eq!(
        fs::read(&victim).unwrap(),
        b"victim bytes that must survive",
        "the hard link's target was written through"
    );
    assert_eq!(fs::read(root.path().join(relative)).unwrap(), b"payload");
    assert!(!staged.exists());
}

#[test]
#[cfg(any(windows, unix))]
fn a_dangling_file_link_at_the_stage_path_does_not_create_its_target() {
    // A file symlink whose target does not exist: a plain `File::create`
    // follows it and creates the target outside the managed root. On Windows
    // creating file symlinks needs a privilege most accounts lack, so this
    // test skips where the link cannot be made and only then asserts nothing.
    let root = ManagedRoot::new();
    let relative = "Products/product-1--aa.md";
    fs::create_dir_all(root.path().join("Products")).unwrap();
    let outside = std::env::temp_dir().join(format!(
        "pmc-outside-file-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let staged = stage_path(root.path(), relative, "t4");
    if create_file_link(&outside, &staged).is_err() {
        return;
    }

    let outcome = publish_managed_file(root.path(), Path::new(relative), b"payload", "t4");

    assert!(
        matches!(outcome, Err(ManagedPublicationError::PublicationFailed)),
        "publication proceeded through a planted stage link: {outcome:?}"
    );
    assert!(
        !outside.exists(),
        "the link's target was created outside the root"
    );
    assert!(!root.path().join(relative).exists());
    let _ = fs::remove_file(&staged);
}

#[test]
fn a_leftover_regular_stage_file_from_a_crashed_run_is_replaced() {
    // A crash between staging and rename can leave a plain stage file for
    // the same token. That must not block the retry that finishes the work:
    // it is a regular file, not a link, so it is replaced.
    let root = ManagedRoot::new();
    let relative = "Products/product-1--aa.md";
    fs::create_dir_all(root.path().join("Products")).unwrap();
    let staged = stage_path(root.path(), relative, "t1");
    fs::write(&staged, b"half-written garbage").unwrap();

    publish_managed_file(root.path(), Path::new(relative), b"final bytes", "t1")
        .expect("a retry over a plain leftover stage must finish");

    assert_eq!(
        fs::read(root.path().join(relative)).unwrap(),
        b"final bytes"
    );
    assert!(!staged.exists(), "the stage must not linger after rename");
}

#[test]
fn an_existing_directory_at_the_stage_path_is_refused() {
    // Not a link and not a regular file: there is no honest way to stage
    // through it, so publication fails rather than removing a directory.
    let root = ManagedRoot::new();
    let relative = "Products/product-1--aa.md";
    fs::create_dir_all(root.path().join("Products")).unwrap();
    let staged = stage_path(root.path(), relative, "t2");
    fs::create_dir_all(&staged).unwrap();

    let outcome = publish_managed_file(root.path(), Path::new(relative), b"payload", "t2");

    assert!(matches!(
        outcome,
        Err(ManagedPublicationError::PublicationFailed)
    ));
    assert!(
        staged.is_dir(),
        "a directory at the stage path must be left alone"
    );
    assert!(!root.path().join(relative).exists());
}
