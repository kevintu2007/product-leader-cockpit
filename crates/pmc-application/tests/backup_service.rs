//! The file-backed side of backup runs: the named bootstrap is audited once,
//! and the startup check trusts only archives that are byte for byte what
//! was verified.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_application::backup_service::BackupService;
use pmc_application::backup_service::{UnopenedLedger, UpgradeFailure};
use pmc_application::operational_backup::BackupError;
use pmc_domain::time::UtcTimestamp;
use pmc_ledger::sqlite::{inspect_ledger, LedgerCompatibility, UpgradeOutcome, UpgradeSource};
use pmc_ledger::sqlite::{LedgerSnapshotManifest, SqliteProductLedger};
use pmc_platform::backup_archive::Passphrase;
use pmc_platform::host_audit::{
    AuditOutcome, AuditWorkspace, HostAuditLog, BOOTSTRAP_EMPTY_AUTHORITY,
};
use pmc_platform::settings::CanonicalDirectoryPath;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("pmc-backup-service-{name}-{nonce}-{sequence}"));
    fs::create_dir_all(&dir).unwrap_or_else(|error| panic!("scratch dir failed: {error}"));
    fs::canonicalize(&dir).unwrap_or_else(|error| panic!("canonicalize failed: {error}"))
}

const NOW: i64 = 1_790_000_000_000;

fn clock() -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(NOW)
}

#[test]
fn the_bootstrap_of_a_pristine_live_ledger_is_audited_once() {
    let root = scratch("bootstrap");
    let ledger = SqliteProductLedger::open(root.join("ledger.sqlite3"))
        .unwrap_or_else(|error| panic!("open failed: {error:?}"));
    let service = BackupService::in_directory(&root, "live", AuditWorkspace::Live, None);
    service.record_bootstrap(&ledger, clock());
    service.record_bootstrap(&ledger, clock());
    let events = HostAuditLog::new(root.join("host-audit-v1.jsonl"))
        .events()
        .unwrap_or_else(|error| panic!("{error}"));
    let bootstraps: Vec<_> = events
        .iter()
        .filter(|event| event.event_code == BOOTSTRAP_EMPTY_AUTHORITY)
        .collect();
    assert_eq!(bootstraps.len(), 1);
    assert_eq!(bootstraps[0].outcome, AuditOutcome::Succeeded);
}

#[test]
fn training_records_no_bootstrap() {
    let root = scratch("training");
    let ledger = SqliteProductLedger::open(root.join("ledger.sqlite3"))
        .unwrap_or_else(|error| panic!("open failed: {error:?}"));
    let service = BackupService::in_directory(&root, "training", AuditWorkspace::Training, None);
    service.record_bootstrap(&ledger, clock());
    assert!(HostAuditLog::new(root.join("host-audit-v1.jsonl"))
        .events()
        .unwrap_or_else(|error| panic!("{error}"))
        .is_empty());
}

fn back_up(service: &BackupService, ledger: &SqliteProductLedger, destination: &Path, id: &str) {
    let destination = CanonicalDirectoryPath::new(destination.to_path_buf())
        .unwrap_or_else(|error| panic!("destination failed: {error}"));
    let snapshot = service
        .snapshot(ledger)
        .unwrap_or_else(|error| panic!("snapshot failed: {error:?}"));
    let passphrase = Passphrase::new("correct horse battery staple river".to_owned());
    let record = service
        .publish(
            &snapshot,
            b"{}",
            &destination,
            &passphrase,
            id,
            &clock,
            &|path: &Path, manifest: &LedgerSnapshotManifest| {
                ledger.verify_snapshot(path, manifest).is_ok()
            },
        )
        .unwrap_or_else(|error| panic!("publish failed: {error:?}"));
    service
        .register(&record)
        .unwrap_or_else(|error| panic!("register failed: {error}"));
}

#[test]
fn the_startup_check_trusts_only_unaltered_archives_and_counts_orphans() {
    let root = scratch("reconcile");
    let destination = root.join("backups");
    fs::create_dir(&destination).unwrap_or_else(|error| panic!("{error}"));
    let ledger = SqliteProductLedger::open(root.join("ledger.sqlite3"))
        .unwrap_or_else(|error| panic!("open failed: {error:?}"));
    let service = BackupService::in_directory(&root, "live", AuditWorkspace::Live, None);
    back_up(&service, &ledger, &destination, "a1a1a1a1");
    let folder =
        CanonicalDirectoryPath::new(destination.clone()).unwrap_or_else(|error| panic!("{error}"));

    let found = service.reconcile(Some(&folder));
    assert_eq!(found.valid_verified_at, vec![NOW]);
    assert_eq!(found.orphan_count, 0);

    // An archive copied in from elsewhere is an orphan, never trusted.
    fs::write(
        destination.join("pmc-operational-copied-v1.tar.zst.age"),
        b"x",
    )
    .unwrap_or_else(|error| panic!("{error}"));
    // And the recorded one, altered, is no longer valid.
    let archive = fs::read_dir(&destination)
        .unwrap_or_else(|error| panic!("{error}"))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| !path.to_string_lossy().contains("copied"))
        .unwrap_or_else(|| panic!("no archive"));
    let mut bytes = fs::read(&archive).unwrap_or_else(|error| panic!("{error}"));
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    fs::write(&archive, bytes).unwrap_or_else(|error| panic!("{error}"));

    let found = service.reconcile(Some(&folder));
    assert!(found.valid_verified_at.is_empty());
    assert_eq!(found.orphan_count, 1);
}

/// A copy of the frozen, populated v46 Ledger in a fresh root, and a service
/// that owns it.
fn fixture_service(label: &str) -> (PathBuf, PathBuf, BackupService) {
    let root = scratch(label);
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("pmc-ledger")
        .join("tests")
        .join("fixtures")
        .join("ledger-v46-seeded.sqlite3");
    let ledger = root.join("product-ledger.sqlite3");
    fs::copy(fixture, &ledger).unwrap_or_else(|error| panic!("{error}"));
    let destination = root.join("backups");
    fs::create_dir(&destination).unwrap_or_else(|error| panic!("{error}"));
    let service =
        BackupService::in_directory(&root, "live", AuditWorkspace::Live, Some(ledger.clone()));
    (ledger, destination, service)
}

fn source_of(service: &BackupService) -> UpgradeSource {
    match service.inspect().unwrap_or_else(|error| panic!("{error}")) {
        LedgerCompatibility::Current(inspection)
        | LedgerCompatibility::UpgradeableFrom(inspection) => UpgradeSource::from(&inspection),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn a_pre_upgrade_backup_is_bound_to_the_ledger_it_copies() {
    let (ledger, destination, service) = fixture_service("pre-upgrade");
    let expected = source_of(&service);
    let folder =
        CanonicalDirectoryPath::new(destination.clone()).unwrap_or_else(|error| panic!("{error}"));
    let passphrase = Passphrase::new("correct horse battery staple river".to_owned());
    let (record, receipt) = service
        .back_up_before_upgrade(&expected, b"{}", &folder, &passphrase, "b0b0b0b0", &clock)
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(receipt.source(), &expected);
    assert_eq!(receipt.archive_id(), record.archive_id);
    assert_eq!(record.authority_inventory_sha256, expected.inventory_sha256);
    assert_eq!(record.authority_record_count, 139);
    // Registered like any backup: it counts as the day's backup.
    assert_eq!(
        service.reconcile(Some(&folder)).valid_verified_at,
        vec![NOW]
    );
    // On its receipt the frozen v46 Ledger upgrades to this binary's v48,
    // every record intact; asked again, it is current and untouched.
    assert_eq!(
        service.upgrade(&expected, receipt),
        Ok(UpgradeOutcome::Upgraded { from: 46, to: 48 })
    );
    let LedgerCompatibility::Current(after) =
        inspect_ledger(&ledger).unwrap_or_else(|error| panic!("{error}"))
    else {
        panic!("not current after the upgrade");
    };
    assert_eq!(after.inventory.sha256, expected.inventory_sha256);
    assert_eq!(after.inventory.record_count, 139);
    assert!(matches!(
        service.inspect(),
        Ok(LedgerCompatibility::Current(_))
    ));
}

#[test]
fn a_ledger_that_changed_since_inspection_is_not_backed_up_for_an_upgrade() {
    let (_ledger, destination, service) = fixture_service("pre-upgrade-changed");
    let expected = UpgradeSource {
        inventory_sha256: "0".repeat(64),
        ..source_of(&service)
    };
    let folder =
        CanonicalDirectoryPath::new(destination.clone()).unwrap_or_else(|error| panic!("{error}"));
    let passphrase = Passphrase::new("correct horse battery staple river".to_owned());
    let result =
        service.back_up_before_upgrade(&expected, b"{}", &folder, &passphrase, "c1c1c1c1", &clock);
    assert!(
        matches!(result, Err(BackupError::SourceChanged)),
        "{result:?}"
    );
    assert!(fs::read_dir(&destination)
        .unwrap_or_else(|error| panic!("{error}"))
        .next()
        .is_none());
}

#[test]
fn an_upgrade_is_refused_when_its_backup_is_gone() {
    let (_ledger, destination, service) = fixture_service("pre-upgrade-gone");
    let expected = source_of(&service);
    let folder =
        CanonicalDirectoryPath::new(destination.clone()).unwrap_or_else(|error| panic!("{error}"));
    let passphrase = Passphrase::new("correct horse battery staple river".to_owned());
    let (record, receipt) = service
        .back_up_before_upgrade(&expected, b"{}", &folder, &passphrase, "d2d2d2d2", &clock)
        .unwrap_or_else(|error| panic!("{error:?}"));
    fs::remove_file(destination.join(&record.file_name)).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        service.upgrade(&expected, receipt),
        Err(UpgradeFailure::BackupNoLongerValid)
    );
}

/// Set the SQLite header's `user_version` (offset 60, big-endian) of a
/// closed Ledger: how a Ledger from another PMC version looks to this one.
fn stamp_user_version(ledger: &Path, version: u32) {
    use std::io::{Seek, SeekFrom, Write};
    let mut file = fs::OpenOptions::new()
        .write(true)
        .open(ledger)
        .unwrap_or_else(|error| panic!("{error}"));
    file.seek(SeekFrom::Start(60))
        .unwrap_or_else(|error| panic!("{error}"));
    file.write_all(&version.to_be_bytes())
        .unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn a_ledger_that_does_not_open_is_classified_for_the_upgrade_gate() {
    let (ledger, _destination, service) = fixture_service("classify");
    // The frozen v46 fixture: the upgrade screen's facts.
    let plan = service
        .upgrade_plan()
        .unwrap_or_else(|| panic!("a v46 Ledger has an upgrade plan"));
    assert_eq!(
        (plan.from_schema, plan.to_schema, plan.record_count),
        (46, 48, 139)
    );
    assert_eq!(service.classify_unopened(), UnopenedLedger::UpgradeRequired);
    stamp_user_version(&ledger, 99);
    assert_eq!(service.classify_unopened(), UnopenedLedger::NewerVersion);
    assert_eq!(service.upgrade_plan(), None);
    stamp_user_version(&ledger, 12);
    assert_eq!(service.classify_unopened(), UnopenedLedger::UnsupportedOld);
    assert_eq!(service.upgrade_plan(), None);
}
