use std::cell::RefCell;

use pmc_platform::uri_launcher::{LaunchError, ObsidianUri, ObsidianUriError, UriLaunchPort};

#[test]
fn the_constructed_uri_always_uses_the_obsidian_open_scheme() {
    let uri = ObsidianUri::open("Synthetic Vault", "notes.md").unwrap();
    assert!(uri.as_str().starts_with("obsidian://open?vault="));
}

#[test]
fn open_encodes_spaces_in_both_vault_and_file_values() {
    let uri = ObsidianUri::open("Synthetic Vault", "Research Notes.md").unwrap();
    assert_eq!(
        uri.as_str(),
        "obsidian://open?vault=Synthetic%20Vault&file=Research%20Notes.md"
    );
}

#[test]
fn open_encodes_zh_tw_characters() {
    let uri = ObsidianUri::open("產品保管庫", "研究/競品筆記.md").unwrap();
    // UTF-8 percent-encoding: every non-ASCII byte becomes %XX; the ASCII
    // '/' path separator and '.' stay literal.
    assert_eq!(
        uri.as_str(),
        "obsidian://open?vault=%E7%94%A2%E5%93%81%E4%BF%9D%E7%AE%A1%E5%BA%AB&file=%E7%A0%94%E7%A9%B6/%E7%AB%B6%E5%93%81%E7%AD%86%E8%A8%98.md"
    );
}

#[test]
fn open_encodes_reserved_symbols_like_ampersand_and_percent() {
    let uri = ObsidianUri::open("A&B", "100% done.md").unwrap();
    assert_eq!(
        uri.as_str(),
        "obsidian://open?vault=A%26B&file=100%25%20done.md"
    );
}

#[test]
fn open_preserves_nested_folder_slashes_in_the_file_value() {
    let uri = ObsidianUri::open("Vault", "Research/Competitive/notes.md").unwrap();
    assert_eq!(
        uri.as_str(),
        "obsidian://open?vault=Vault&file=Research/Competitive/notes.md"
    );
}

#[test]
fn open_encodes_a_fragment_marker_in_the_file_value() {
    let uri = ObsidianUri::open("Vault", "notes.md#Section 1").unwrap();
    assert_eq!(
        uri.as_str(),
        "obsidian://open?vault=Vault&file=notes.md%23Section%201"
    );
}

#[test]
fn open_rejects_an_empty_vault_name() {
    let result = ObsidianUri::open("", "notes.md");
    assert_eq!(result, Err(ObsidianUriError::EmptyVaultName));
}

#[test]
fn open_rejects_an_empty_file_path() {
    let result = ObsidianUri::open("Vault", "");
    assert_eq!(result, Err(ObsidianUriError::EmptyFilePath));
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
fn a_fake_launch_port_records_the_launched_uri() {
    let launcher = FakeUriLauncher::new();
    let uri = ObsidianUri::open("Vault", "notes.md").unwrap();
    launcher.launch(&uri).unwrap();
    assert_eq!(launcher.launched.borrow().as_slice(), [uri.as_str()]);
}

#[test]
fn a_fake_launch_port_can_simulate_an_unavailable_handler() {
    let launcher = FakeUriLauncher::failing(LaunchError::Unavailable);
    let uri = ObsidianUri::open("Vault", "notes.md").unwrap();
    let result = launcher.launch(&uri);
    assert_eq!(result, Err(LaunchError::Unavailable));
}
