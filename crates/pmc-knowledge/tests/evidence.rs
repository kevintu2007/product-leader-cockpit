use std::fs;
use std::path::PathBuf;
#[cfg(windows)]
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_domain::evidence::VaultRelativePath;
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::{EvidenceVerification, IntegrityDigest};
use pmc_knowledge::evidence::observe_evidence;
use pmc_knowledge::vault::VaultRoot;

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn isolated_dir(test_name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "pmc-synthetic-evidence-observe-{test_name}-{nonce}-{sequence}"
    ));
    fs::create_dir_all(&path).unwrap_or_else(|error| panic!("fixture setup failed: {error}"));
    fs::canonicalize(&path).unwrap_or_else(|error| panic!("fixture canonicalize failed: {error}"))
}

fn digest_of(content: &[u8]) -> IntegrityDigest {
    use sha2::{Digest, Sha256};
    IntegrityDigest::parse(format!("{:x}", Sha256::digest(content))).unwrap()
}

#[test]
fn a_readable_file_with_no_pinned_fingerprint_is_observed_unpinned_never_verified() {
    // With nothing pinned there is nothing to verify against. The
    // observation still records what was read -- the digest and the time --
    // so a later change to the file is visible as a new observation.
    let root = isolated_dir("no-expected");
    let vault = VaultRoot::validate(&root).unwrap();
    fs::write(root.join("notes.md"), b"synthetic content")
        .unwrap_or_else(|error| panic!("fixture setup failed: {error}"));
    let relative = VaultRelativePath::parse("notes.md").unwrap();

    let outcome = observe_evidence(
        &vault,
        &relative,
        None,
        &EvidenceVerification::Unverified,
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap();

    match outcome {
        EvidenceVerification::ObservedUnpinned {
            observed_at,
            integrity_digest,
        } => {
            assert_eq!(observed_at, UtcTimestamp::from_unix_millis(1_000));
            assert_eq!(
                integrity_digest.as_str(),
                digest_of(b"synthetic content").as_str()
            );
        }
        other => panic!("an unpinned reference must never verify; got {other:?}"),
    }
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn a_changed_unpinned_file_is_a_new_observation_not_a_mismatch_and_not_a_verification() {
    let root = isolated_dir("unpinned-changed");
    let vault = VaultRoot::validate(&root).unwrap();
    fs::write(root.join("notes.md"), b"first content")
        .unwrap_or_else(|error| panic!("fixture setup failed: {error}"));
    let relative = VaultRelativePath::parse("notes.md").unwrap();
    let first = observe_evidence(
        &vault,
        &relative,
        None,
        &EvidenceVerification::Unverified,
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap();

    fs::write(root.join("notes.md"), b"second content")
        .unwrap_or_else(|error| panic!("fixture mutation failed: {error}"));
    let second = observe_evidence(
        &vault,
        &relative,
        None,
        &first,
        UtcTimestamp::from_unix_millis(2_000),
    )
    .unwrap();

    // Without a pin, a change is not a mismatch: there was no baseline to
    // mismatch against. It is also not silently equal to the first
    // observation, which is what would let the application's Unchanged
    // short-circuit swallow it.
    assert_ne!(second, first);
    match second {
        EvidenceVerification::ObservedUnpinned {
            observed_at,
            integrity_digest,
        } => {
            assert_eq!(observed_at, UtcTimestamp::from_unix_millis(2_000));
            assert_eq!(
                integrity_digest.as_str(),
                digest_of(b"second content").as_str()
            );
        }
        other => panic!("expected a fresh ObservedUnpinned, got {other:?}"),
    }
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn a_missing_file_previously_observed_unpinned_becomes_unverified_not_degraded() {
    // Degraded carries a *verified* baseline forward. An unpinned observation
    // never had one, so all that is left to say is that the file is
    // unreadable.
    let root = isolated_dir("unpinned-missing");
    let vault = VaultRoot::validate(&root).unwrap();
    let relative = VaultRelativePath::parse("missing.md").unwrap();
    let previous = EvidenceVerification::ObservedUnpinned {
        observed_at: UtcTimestamp::from_unix_millis(1_000),
        integrity_digest: digest_of(b"anything"),
    };

    let outcome = observe_evidence(
        &vault,
        &relative,
        None,
        &previous,
        UtcTimestamp::from_unix_millis(3_000),
    )
    .unwrap();

    assert!(matches!(outcome, EvidenceVerification::Unverified));
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn a_file_matching_its_pinned_fingerprint_becomes_verified() {
    let root = isolated_dir("matching");
    let vault = VaultRoot::validate(&root).unwrap();
    let content: &[u8] = b"pinned content";
    fs::write(root.join("notes.md"), content)
        .unwrap_or_else(|error| panic!("fixture setup failed: {error}"));
    let relative = VaultRelativePath::parse("notes.md").unwrap();
    let expected = digest_of(content);

    let outcome = observe_evidence(
        &vault,
        &relative,
        Some(&expected),
        &EvidenceVerification::Unverified,
        UtcTimestamp::from_unix_millis(2_000),
    )
    .unwrap();

    match outcome {
        EvidenceVerification::Verified {
            verified_at,
            integrity_digest,
        } => {
            assert_eq!(verified_at, UtcTimestamp::from_unix_millis(2_000));
            assert_eq!(integrity_digest.as_str(), expected.as_str());
        }
        other => panic!("expected Verified, got {other:?}"),
    }
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn a_file_not_matching_its_pinned_fingerprint_is_an_integrity_mismatch() {
    let root = isolated_dir("mismatch");
    let vault = VaultRoot::validate(&root).unwrap();
    fs::write(root.join("notes.md"), b"changed content")
        .unwrap_or_else(|error| panic!("fixture setup failed: {error}"));
    let relative = VaultRelativePath::parse("notes.md").unwrap();
    let expected = digest_of(b"original content");

    let outcome = observe_evidence(
        &vault,
        &relative,
        Some(&expected),
        &EvidenceVerification::Unverified,
        UtcTimestamp::from_unix_millis(2_000),
    )
    .unwrap();

    assert!(matches!(outcome, EvidenceVerification::IntegrityMismatch));
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn a_missing_file_never_verified_stays_unverified() {
    let root = isolated_dir("missing-unverified");
    let vault = VaultRoot::validate(&root).unwrap();
    let relative = VaultRelativePath::parse("missing.md").unwrap();

    let outcome = observe_evidence(
        &vault,
        &relative,
        None,
        &EvidenceVerification::Unverified,
        UtcTimestamp::from_unix_millis(3_000),
    )
    .unwrap();

    assert!(matches!(outcome, EvidenceVerification::Unverified));
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn a_missing_file_previously_verified_degrades_and_keeps_its_original_verified_at() {
    let root = isolated_dir("missing-degrade");
    let vault = VaultRoot::validate(&root).unwrap();
    let relative = VaultRelativePath::parse("missing.md").unwrap();
    let original_digest = digest_of(b"was here once");
    let previous = EvidenceVerification::Verified {
        verified_at: UtcTimestamp::from_unix_millis(500),
        integrity_digest: original_digest.clone(),
    };

    let outcome = observe_evidence(
        &vault,
        &relative,
        Some(&original_digest),
        &previous,
        UtcTimestamp::from_unix_millis(9_999),
    )
    .unwrap();

    match outcome {
        EvidenceVerification::DegradedLastVerified {
            last_verified_at,
            integrity_digest,
        } => {
            assert_eq!(last_verified_at, UtcTimestamp::from_unix_millis(500));
            assert_eq!(integrity_digest.as_str(), original_digest.as_str());
        }
        other => panic!("expected DegradedLastVerified, got {other:?}"),
    }
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn a_missing_file_already_degraded_keeps_the_same_original_verified_at_not_now() {
    let root = isolated_dir("missing-still-degraded");
    let vault = VaultRoot::validate(&root).unwrap();
    let relative = VaultRelativePath::parse("missing.md").unwrap();
    let original_digest = digest_of(b"was here once");
    let previous = EvidenceVerification::DegradedLastVerified {
        last_verified_at: UtcTimestamp::from_unix_millis(500),
        integrity_digest: original_digest.clone(),
    };

    let outcome = observe_evidence(
        &vault,
        &relative,
        Some(&original_digest),
        &previous,
        UtcTimestamp::from_unix_millis(9_999),
    )
    .unwrap();

    assert_eq!(outcome, previous);
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn a_missing_file_previously_an_integrity_mismatch_becomes_unverified() {
    let root = isolated_dir("missing-was-mismatch");
    let vault = VaultRoot::validate(&root).unwrap();
    let relative = VaultRelativePath::parse("missing.md").unwrap();

    let outcome = observe_evidence(
        &vault,
        &relative,
        None,
        &EvidenceVerification::IntegrityMismatch,
        UtcTimestamp::from_unix_millis(9_999),
    )
    .unwrap();

    assert!(matches!(outcome, EvidenceVerification::Unverified));
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn a_path_escaping_through_a_reparse_point_component_is_rejected() {
    let root = isolated_dir("reparse-escape");
    let external = isolated_dir("reparse-escape-external");
    fs::write(external.join("secret.md"), b"outside the vault")
        .unwrap_or_else(|error| panic!("fixture setup failed: {error}"));
    let link = root.join("Linked");
    create_directory_link(&external, &link)
        .unwrap_or_else(|error| panic!("link setup failed: {error}"));
    let vault = VaultRoot::validate(&root).unwrap();
    let relative = VaultRelativePath::parse("Linked/secret.md").unwrap();

    let result = observe_evidence(
        &vault,
        &relative,
        None,
        &EvidenceVerification::Unverified,
        UtcTimestamp::from_unix_millis(1_000),
    );

    assert!(result.is_err());
    fs::remove_dir_all(&root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
    fs::remove_dir_all(&external).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
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
