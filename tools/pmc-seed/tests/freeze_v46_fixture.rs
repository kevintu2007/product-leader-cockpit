//! Writes the frozen, populated v46 Ledger that the in-place upgrade is
//! tested against (slice 8d). Run once, by hand, at a commit whose binary is
//! v46:
//!
//! ```text
//! PMC_WRITE_V46_FIXTURE=1 cargo test -p pmc-seed --test freeze_v46_fixture -- --ignored
//! ```
//!
//! The fixture is created by this binary's own `SqliteProductLedger::open`
//! (the real v1..v46 descriptors and bootstrap) and filled through the real
//! writers of the Training seed — never by writing a current schema and
//! setting `user_version`. The provenance file beside it records the commit,
//! the schema version, the file's SHA-256 and the authority inventory, and
//! `pmc-ledger`'s tests check both on every run.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;

use pmc_ledger::sqlite::{inspect_ledger, LedgerCompatibility, CURRENT_SCHEMA_VERSION};
use pmc_platform::settings::ProtectedSettingsRoot;
use pmc_seed::{seed_training_in, SeedOptions, LEDGER_FILE_NAME};
use sha2::{Digest, Sha256};

#[cfg(windows)]
fn ensure_test_app_data_root_is_not_redirected() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("pmc-seed-test-app-data");
    fs::create_dir_all(&root).unwrap();
    std::env::set_var("LOCALAPPDATA", fs::canonicalize(&root).unwrap());
}

#[cfg(not(windows))]
fn ensure_test_app_data_root_is_not_redirected() {}

#[test]
#[ignore = "writes the checked-in v46 fixture; run by hand with PMC_WRITE_V46_FIXTURE=1"]
fn write_frozen_v46_fixture() {
    if std::env::var("PMC_WRITE_V46_FIXTURE").as_deref() != Ok("1") {
        return;
    }
    assert_eq!(
        CURRENT_SCHEMA_VERSION, 46,
        "the fixture must come from a v46 binary"
    );
    ensure_test_app_data_root_is_not_redirected();
    let application = format!(
        "PmcSeedV46Fixture-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let report = seed_training_in(&application, SeedOptions::default()).unwrap();
    let source = report.training_root.join(LEDGER_FILE_NAME);
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = source.as_os_str().to_owned();
        sidecar.push(suffix);
        assert!(
            fs::metadata(&sidecar).map_or(true, |metadata| metadata.len() == 0),
            "the seeded Ledger must be closed cleanly ({suffix})"
        );
    }
    let LedgerCompatibility::Current(inspection) = inspect_ledger(&source).unwrap() else {
        panic!("the seeded Ledger must inspect as current v46");
    };

    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("crates")
        .join("pmc-ledger")
        .join("tests")
        .join("fixtures");
    fs::create_dir_all(&fixtures).unwrap();
    let target = fixtures.join("ledger-v46-seeded.sqlite3");
    fs::copy(&source, &target).unwrap();
    let bytes = fs::read(&target).unwrap();
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|text| text.trim().to_owned())
        .unwrap_or_default();
    let provenance = format!(
        "{{\n  \"format\": \"pmc-ledger-fixture-provenance/v1\",\n  \"file\": \"ledger-v46-seeded.sqlite3\",\n  \"source_commit\": \"{commit}\",\n  \"creator\": \"SqliteProductLedger::open + pmc-seed Training scenario\",\n  \"schema_version\": {},\n  \"ledger_revision\": {},\n  \"fixture_sha256\": \"{:x}\",\n  \"authority_inventory_sha256\": \"{}\",\n  \"authority_record_count\": {}\n}}\n",
        inspection.schema_version,
        inspection.ledger_revision,
        Sha256::digest(&bytes),
        inspection.inventory.sha256,
        inspection.inventory.record_count,
    );
    fs::write(
        fixtures.join("ledger-v46-seeded.provenance.json"),
        provenance,
    )
    .unwrap();

    if let Ok(root) = ProtectedSettingsRoot::prepare(&application) {
        let _ = fs::remove_dir_all(root.path());
    }
}
