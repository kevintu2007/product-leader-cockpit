use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_platform::filesystem::{compute_sha256_fingerprint, FilesystemError};

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn isolated_dir(test_name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "pmc-synthetic-filesystem-{test_name}-{nonce}-{sequence}"
    ));
    fs::create_dir_all(&path).unwrap_or_else(|error| panic!("fixture setup failed: {error}"));
    path
}

#[test]
fn a_known_file_produces_its_known_sha256_digest() {
    let root = isolated_dir("known-digest");
    let file = root.join("notes.md");
    fs::write(&file, b"synthetic evidence content")
        .unwrap_or_else(|error| panic!("fixture setup failed: {error}"));

    let digest = compute_sha256_fingerprint(&file).unwrap();
    assert_eq!(
        digest,
        "547c0b0b5ede066ab248045c3a33383229f489f734198d08db7742e4c32638e5"
    );
    assert_eq!(digest.len(), 64);
    assert!(digest
        .chars()
        .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase()));

    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn an_empty_file_produces_the_known_empty_digest() {
    let root = isolated_dir("empty-digest");
    let file = root.join("empty.md");
    fs::write(&file, b"").unwrap_or_else(|error| panic!("fixture setup failed: {error}"));

    let digest = compute_sha256_fingerprint(&file).unwrap();
    assert_eq!(
        digest,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(digest.len(), 64);

    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn identical_content_produces_the_same_digest_and_different_content_differs() {
    let root = isolated_dir("stability-digest");
    let first = root.join("first.md");
    let second = root.join("second.md");
    let third = root.join("third.md");
    fs::write(&first, b"shared content").unwrap_or_else(|error| panic!("setup failed: {error}"));
    fs::write(&second, b"shared content").unwrap_or_else(|error| panic!("setup failed: {error}"));
    fs::write(&third, b"different content").unwrap_or_else(|error| panic!("setup failed: {error}"));

    let first_digest = compute_sha256_fingerprint(&first).unwrap();
    let second_digest = compute_sha256_fingerprint(&second).unwrap();
    let third_digest = compute_sha256_fingerprint(&third).unwrap();

    assert_eq!(first_digest, second_digest);
    assert_ne!(first_digest, third_digest);

    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn a_missing_file_is_reported_as_not_found() {
    let root = isolated_dir("missing-file");
    let missing = root.join("does-not-exist.md");
    let result = compute_sha256_fingerprint(&missing);
    assert_eq!(result, Err(FilesystemError::NotFound));
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn a_directory_is_rejected_as_not_a_regular_file() {
    let root = isolated_dir("directory-target");
    let result = compute_sha256_fingerprint(&root);
    assert_eq!(result, Err(FilesystemError::NotARegularFile));
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn a_large_file_over_the_streaming_buffer_size_hashes_correctly() {
    let root = isolated_dir("large-file");
    let file = root.join("large.bin");
    let content = vec![b'a'; 200_000];
    fs::write(&file, &content).unwrap_or_else(|error| panic!("fixture setup failed: {error}"));

    let digest = compute_sha256_fingerprint(&file).unwrap();
    let repeated_small = {
        let root = isolated_dir("large-file-cross-check");
        let file = root.join("small.bin");
        fs::write(&file, &content).unwrap_or_else(|error| panic!("setup failed: {error}"));
        let value = compute_sha256_fingerprint(&file).unwrap();
        fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
        value
    };
    assert_eq!(digest, repeated_small);

    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}
