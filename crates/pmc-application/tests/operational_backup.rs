//! An Operational Backup end to end (S7-A, ADR 0010): a real Ledger snapshot,
//! the encrypted archive written beside the destination, decrypted and
//! checked again, the snapshot reopened, then renamed into place — and on any
//! failure, nothing published and no plaintext left behind.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_application::operational_backup::{
    archive_file_name, clear_work_area, publish_backup, snapshot_ledger, BackupError, PublishInput,
};
use pmc_domain::time::UtcTimestamp;
use pmc_ledger::sqlite::{LedgerSnapshotManifest, SqliteProductLedger};
use pmc_platform::backup_archive::{read_archive, Passphrase};
use pmc_platform::backup_registry::{check_record, RecordStatus, ARCHIVE_PREFIX, ARCHIVE_SUFFIX};
use pmc_platform::settings::CanonicalDirectoryPath;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("pmc-opbackup-{name}-{nonce}-{sequence}"));
    fs::create_dir_all(&dir).unwrap_or_else(|error| panic!("scratch dir failed: {error}"));
    fs::canonicalize(&dir).unwrap_or_else(|error| panic!("canonicalize failed: {error}"))
}

struct Setup {
    ledger: SqliteProductLedger,
    destination: CanonicalDirectoryPath,
    work: PathBuf,
}

fn setup(name: &str) -> Setup {
    let root = scratch(name);
    let ledger = SqliteProductLedger::open(root.join("product-ledger.sqlite3"))
        .unwrap_or_else(|error| panic!("ledger open failed: {error:?}"));
    let destination_dir = root.join("backups");
    fs::create_dir(&destination_dir).unwrap_or_else(|error| panic!("mkdir failed: {error}"));
    let destination = CanonicalDirectoryPath::new(destination_dir)
        .unwrap_or_else(|error| panic!("destination failed: {error}"));
    Setup {
        ledger,
        destination,
        work: root.join("backup-work"),
    }
}

const NOW: i64 = 1_790_000_000_000;

fn clock() -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(NOW)
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[test]
fn a_backup_is_published_verified_and_registered_with_nothing_left_behind() {
    let setup = setup("happy");
    let snapshot = snapshot_ledger(&setup.ledger, &setup.work)
        .unwrap_or_else(|error| panic!("snapshot failed: {error:?}"));
    let passphrase = Passphrase::new("correct horse battery staple river".to_owned());
    let verify = |path: &Path, expected: &LedgerSnapshotManifest| {
        setup.ledger.verify_snapshot(path, expected).is_ok()
    };
    let record = publish_backup(PublishInput {
        snapshot: &snapshot,
        settings_json: br#"{"format":"pmc-backup-settings/v1"}"#,
        destination: &setup.destination,
        work: &setup.work,
        passphrase: &passphrase,
        archive_id: "a1b2c3d4",
        clock: &clock,
        verify_ledger: &verify,
    })
    .unwrap_or_else(|error| panic!("publish failed: {error:?}"));

    // Exactly one file in the destination: the final archive, no partial.
    let published = entries(setup.destination.as_path());
    assert_eq!(published, vec![record.file_name.clone()]);
    assert!(record.file_name.starts_with(ARCHIVE_PREFIX));
    assert!(record.file_name.ends_with(ARCHIVE_SUFFIX));
    assert_eq!(check_record(&record), RecordStatus::Valid);
    let manifest = &snapshot.manifest;
    assert_eq!(record.ledger_schema_version, manifest.schema_version());
    assert_eq!(record.ledger_revision, manifest.ledger_revision());
    assert_eq!(record.snapshot_sha256, manifest.sha256());
    assert_eq!(record.verified_at_millis, NOW);
    assert_eq!(record.authority_record_count, 0);
    assert_eq!(record.authority_inventory_sha256, snapshot.inventory.sha256);

    // No plaintext anywhere in the work area.
    assert!(
        entries(&setup.work).is_empty(),
        "{:?}",
        entries(&setup.work)
    );

    // And it opens again with the same passphrase.
    let file = fs::File::open(setup.destination.as_path().join(&record.file_name))
        .unwrap_or_else(|error| panic!("open failed: {error}"));
    let verified = read_archive(file, &passphrase, None)
        .unwrap_or_else(|error| panic!("read failed: {error}"));
    let names: Vec<&str> = verified
        .manifest
        .members
        .iter()
        .map(|member| member.name.as_str())
        .collect();
    assert_eq!(
        names,
        ["authority-inventory.txt", "ledger.sqlite3", "settings.json"]
    );
}

#[test]
fn a_backup_whose_ledger_does_not_verify_publishes_nothing() {
    let setup = setup("refused");
    let snapshot = snapshot_ledger(&setup.ledger, &setup.work)
        .unwrap_or_else(|error| panic!("snapshot failed: {error:?}"));
    let passphrase = Passphrase::new("correct horse battery staple river".to_owned());
    let refuse = |_: &Path, _: &LedgerSnapshotManifest| false;
    let result = publish_backup(PublishInput {
        snapshot: &snapshot,
        settings_json: b"{}",
        destination: &setup.destination,
        work: &setup.work,
        passphrase: &passphrase,
        archive_id: "e5f6a7b8",
        clock: &clock,
        verify_ledger: &refuse,
    });
    assert!(
        matches!(result, Err(BackupError::VerificationFailed)),
        "{result:?}"
    );
    assert!(entries(setup.destination.as_path()).is_empty());
    assert!(entries(&setup.work).is_empty());
}

#[test]
fn leftovers_of_an_interrupted_backup_are_cleared() {
    let setup = setup("residue");
    fs::create_dir_all(setup.work.join("verify"))
        .unwrap_or_else(|error| panic!("mkdir failed: {error}"));
    fs::write(
        setup.work.join("verify").join("ledger.sqlite3"),
        b"plaintext",
    )
    .unwrap_or_else(|error| panic!("write failed: {error}"));
    clear_work_area(&setup.work).unwrap_or_else(|error| panic!("clear failed: {error}"));
    assert!(entries(&setup.work).is_empty());
}

#[test]
fn archive_names_carry_the_time_and_the_id() {
    let name = archive_file_name(UtcTimestamp::from_unix_millis(NOW), "A1b2-C3d4!");
    assert!(name.starts_with("pmc-operational-"), "{name}");
    assert!(name.ends_with("-v1.tar.zst.age"), "{name}");
    assert!(name.contains("a1b2c3d4"), "{name}");
    assert!(
        name.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-.".contains(c)),
        "{name}"
    );
}

#[test]
fn an_archive_id_that_would_be_rewritten_is_refused() {
    let setup = setup("bad-id");
    let snapshot = snapshot_ledger(&setup.ledger, &setup.work)
        .unwrap_or_else(|error| panic!("snapshot failed: {error:?}"));
    let passphrase = Passphrase::new("correct horse battery staple river".to_owned());
    let accept = |_: &Path, _: &LedgerSnapshotManifest| true;
    for id in ["A1B2C3D4", "a1b2-c3d4", "short", ""] {
        let result = publish_backup(PublishInput {
            snapshot: &snapshot,
            settings_json: b"{}",
            destination: &setup.destination,
            work: &setup.work,
            passphrase: &passphrase,
            archive_id: id,
            clock: &clock,
            verify_ledger: &accept,
        });
        assert!(
            matches!(result, Err(BackupError::InvalidArchiveId)),
            "{id}: {result:?}"
        );
    }
    assert!(entries(setup.destination.as_path()).is_empty());
    assert!(entries(&setup.work).is_empty());
}

#[test]
fn a_backup_carries_and_rechecks_the_inventory_of_real_records() {
    use pmc_domain::classification::DataClassification;
    use pmc_domain::identity::{AuditEventId, CorrelationId, IdempotencyId, PortfolioId};
    use pmc_domain::portfolio::{CreatePortfolio, LongText, OperationContext, ShortText};
    use pmc_domain::provenance::Provenance;

    let mut setup = setup("records");
    for id in ["synthetic-portfolio-b", "synthetic-portfolio-a"] {
        setup
            .ledger
            .create_portfolio(
                CreatePortfolio {
                    id: PortfolioId::parse(id).unwrap_or_else(|_| panic!("id")),
                    name: ShortText::parse("Synthetic").unwrap_or_else(|_| panic!("name")),
                    details: LongText::parse("Synthetic only.")
                        .unwrap_or_else(|_| panic!("details")),
                    classification: Some(DataClassification::Internal),
                    provenance: Provenance::UserEntered,
                    context: OperationContext {
                        idempotency_id: IdempotencyId::parse(format!("idem-{id}"))
                            .unwrap_or_else(|_| panic!("idem")),
                        correlation_id: CorrelationId::parse(format!("corr-{id}"))
                            .unwrap_or_else(|_| panic!("corr")),
                    },
                },
                AuditEventId::parse(format!("audit-{id}")).unwrap_or_else(|_| panic!("audit")),
                UtcTimestamp::from_unix_millis(1_000),
            )
            .unwrap_or_else(|error| panic!("create failed: {error:?}"));
    }
    let snapshot = snapshot_ledger(&setup.ledger, &setup.work)
        .unwrap_or_else(|error| panic!("snapshot failed: {error:?}"));
    assert_eq!(snapshot.inventory.record_count, 2);
    let passphrase = Passphrase::new("correct horse battery staple river".to_owned());
    let verify = |path: &Path, expected: &LedgerSnapshotManifest| {
        setup.ledger.verify_snapshot(path, expected).is_ok()
    };
    let record = publish_backup(PublishInput {
        snapshot: &snapshot,
        settings_json: b"{}",
        destination: &setup.destination,
        work: &setup.work,
        passphrase: &passphrase,
        archive_id: "c0ffee01",
        clock: &clock,
        verify_ledger: &verify,
    })
    .unwrap_or_else(|error| panic!("publish failed: {error:?}"));
    assert_eq!(record.authority_record_count, 2);
    assert_eq!(record.authority_inventory_sha256, snapshot.inventory.sha256);
    assert_eq!(check_record(&record), RecordStatus::Valid);
}
