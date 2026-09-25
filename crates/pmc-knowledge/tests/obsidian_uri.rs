use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_domain::evidence::VaultRelativePath;
use pmc_knowledge::obsidian_uri::{open_in_obsidian, OpenInObsidianError};
use pmc_knowledge::vault::VaultRoot;
use pmc_platform::uri_launcher::{LaunchError, ObsidianUri, UriLaunchPort};

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn isolated_dir(test_name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "pmc-synthetic-obsidian-uri-{test_name}-{nonce}-{sequence}"
    ));
    fs::create_dir_all(&path).unwrap_or_else(|error| panic!("fixture setup failed: {error}"));
    fs::canonicalize(&path).unwrap_or_else(|error| panic!("fixture canonicalize failed: {error}"))
}

struct FakeUriLauncher {
    launched: RefCell<Vec<String>>,
    fail_with: Option<LaunchError>,
}

impl FakeUriLauncher {
    fn new() -> Self {
        Self {
            launched: RefCell::new(Vec::new()),
            fail_with: None,
        }
    }

    fn failing(error: LaunchError) -> Self {
        Self {
            launched: RefCell::new(Vec::new()),
            fail_with: Some(error),
        }
    }
}

impl UriLaunchPort for FakeUriLauncher {
    fn launch(&self, uri: &ObsidianUri) -> Result<(), LaunchError> {
        if let Some(error) = self.fail_with {
            return Err(error);
        }
        self.launched.borrow_mut().push(uri.as_str().to_owned());
        Ok(())
    }
}

#[test]
fn open_in_obsidian_launches_the_vault_name_and_relative_path_uri() {
    let root = isolated_dir("Synthetic Vault");
    let vault = VaultRoot::validate(&root).unwrap();
    let relative = VaultRelativePath::parse("Research/notes.md").unwrap();
    let launcher = FakeUriLauncher::new();

    open_in_obsidian(&vault, &relative, &launcher).unwrap();

    let vault_name = vault.default_identity_name().unwrap();
    let expected = ObsidianUri::open(&vault_name, "Research/notes.md").unwrap();
    assert_eq!(
        launcher.launched.borrow().as_slice(),
        [expected.as_str().to_owned()]
    );
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn open_in_obsidian_propagates_a_launch_failure() {
    let root = isolated_dir("launch-failure");
    let vault = VaultRoot::validate(&root).unwrap();
    let relative = VaultRelativePath::parse("notes.md").unwrap();
    let launcher = FakeUriLauncher::failing(LaunchError::Unavailable);

    let result = open_in_obsidian(&vault, &relative, &launcher);

    assert_eq!(
        result,
        Err(OpenInObsidianError::Launch(LaunchError::Unavailable))
    );
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn open_in_obsidian_rejects_a_vault_root_with_no_usable_display_name() {
    // A filesystem root has no final path component to use as an Obsidian
    // vault name. `VaultRoot::validate` requires its input already in
    // exact canonical form (Windows: the verbatim `\\?\` form), so the
    // fixture must already be in that form rather than a plain `C:\`.
    let root_path = if cfg!(windows) {
        PathBuf::from(r"\\?\C:\")
    } else {
        PathBuf::from("/")
    };
    let vault = VaultRoot::validate(&root_path).unwrap();
    let relative = VaultRelativePath::parse("notes.md").unwrap();
    let launcher = FakeUriLauncher::new();

    let result = open_in_obsidian(&vault, &relative, &launcher);

    assert_eq!(result, Err(OpenInObsidianError::VaultIdentityUnavailable));
}
