use pmc_domain::evidence::{FingerprintAlgorithm, VaultRelativePath};

#[test]
fn a_relative_vault_path_with_nested_folders_parses() {
    let path = VaultRelativePath::parse("Research/Competitive/2026-08-quarterly-notes.md")
        .expect("relative path should parse");
    assert_eq!(
        path.as_str(),
        "Research/Competitive/2026-08-quarterly-notes.md"
    );
}

#[test]
fn an_empty_path_is_rejected() {
    assert!(VaultRelativePath::parse("").is_err());
    assert!(VaultRelativePath::parse("   ").is_err());
}

#[test]
fn an_absolute_path_is_rejected() {
    assert!(VaultRelativePath::parse("/Research/notes.md").is_err());
}

#[test]
fn a_windows_drive_letter_path_is_rejected() {
    assert!(VaultRelativePath::parse("C:/Research/notes.md").is_err());
    assert!(VaultRelativePath::parse("C:\\Research\\notes.md").is_err());
}

#[test]
fn a_backslash_separated_path_is_rejected() {
    assert!(VaultRelativePath::parse("Research\\notes.md").is_err());
}

#[test]
fn a_traversal_segment_is_rejected() {
    assert!(VaultRelativePath::parse("../secrets.md").is_err());
    assert!(VaultRelativePath::parse("Research/../../secrets.md").is_err());
    assert!(VaultRelativePath::parse("Research/./notes.md").is_err());
}

#[test]
fn an_empty_segment_from_a_double_slash_is_rejected() {
    assert!(VaultRelativePath::parse("Research//notes.md").is_err());
}

#[test]
fn a_control_character_is_rejected() {
    assert!(VaultRelativePath::parse("Research/notes\u{0}.md").is_err());
}

#[test]
fn an_oversized_path_is_rejected() {
    let oversized = format!("{}.md", "a".repeat(400));
    assert!(VaultRelativePath::parse(oversized).is_err());
}

#[test]
fn the_fingerprint_algorithm_round_trips_through_its_persisted_form() {
    assert_eq!(FingerprintAlgorithm::Sha256.as_persisted(), "sha256");
    assert!(matches!(
        FingerprintAlgorithm::from_persisted("sha256"),
        Ok(FingerprintAlgorithm::Sha256)
    ));
}

#[test]
fn an_unknown_persisted_fingerprint_algorithm_is_rejected() {
    assert!(FingerprintAlgorithm::from_persisted("md5").is_err());
}
