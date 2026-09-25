//! Operational Restore end to end on real archives (S7-B1): check, prepare
//! with a recovery backup, approve once, replace with the Ledger closed, and
//! put everything back when anything fails or a crash interrupts.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_application::backup_service::BackupService;
use pmc_application::restore_service::{
    ChosenArchive, RecoveryBackupInput, RecoveryKind, ReopenedLedger, RestoreError,
};
use pmc_domain::classification::DataClassification;
use pmc_domain::identity::{AuditEventId, CorrelationId, IdempotencyId, PortfolioId};
use pmc_domain::portfolio::{CreatePortfolio, LongText, OperationContext, ShortText};
use pmc_domain::provenance::Provenance;
use pmc_domain::time::UtcTimestamp;
use pmc_ledger::sqlite::{inspect_ledger, LedgerCompatibility, SqliteProductLedger};
use pmc_platform::backup_archive::Passphrase;
use pmc_platform::host_audit::AuditWorkspace;
use pmc_platform::local_time::local_calendar_date;
use pmc_platform::restore_control::{
    RestoreControlStore, RestoreOperation, RestoreOutcome, RestorePhase, UnopenedReason,
};
use pmc_platform::settings::{
    CanonicalDirectoryPath, DisplayPatch, ProtectedSettingsRoot, SettingsPatch, SettingsStore,
};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn nonce() -> String {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("{time}-{}", SEQUENCE.fetch_add(1, Ordering::Relaxed))
}

/// This repo's Windows sandbox redirects the real `%LOCALAPPDATA%`; point the
/// test process at a plain directory under `target/`, as the other suites do.
fn protected_root(label: &str) -> ProtectedSettingsRoot {
    #[cfg(windows)]
    {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("target")
                .join("pmc-restore-test-app-data");
            fs::create_dir_all(&root).unwrap_or_else(|error| panic!("{error}"));
            std::env::set_var(
                "LOCALAPPDATA",
                fs::canonicalize(&root).unwrap_or_else(|error| panic!("{error}")),
            );
        });
    }
    ProtectedSettingsRoot::prepare(&format!("PmcRestoreTest-{label}-{}", nonce()))
        .unwrap_or_else(|error| panic!("{error}"))
}

const NOW: i64 = 1_790_049_576_000;
const HOUR: i64 = 60 * 60 * 1000;

fn clock() -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(NOW)
}

fn passphrase() -> Passphrase {
    Passphrase::new("correct horse battery staple river".to_owned())
}

fn add_portfolio(ledger: &mut SqliteProductLedger, id: &str) {
    ledger
        .create_portfolio(
            CreatePortfolio {
                id: PortfolioId::parse(id).unwrap_or_else(|_| panic!("id")),
                name: ShortText::parse("Synthetic").unwrap_or_else(|_| panic!("name")),
                details: LongText::parse("Synthetic only.").unwrap_or_else(|_| panic!("text")),
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
        .unwrap_or_else(|error| panic!("{error:?}"));
}

struct Scene {
    root: ProtectedSettingsRoot,
    ledger: PathBuf,
    service: BackupService,
    settings: SettingsStore,
    folder: CanonicalDirectoryPath,
    archive: PathBuf,
}

/// A live workspace holding one Portfolio, and an archive — made by another
/// workspace — holding two, with Japanese settings.
fn scene(label: &str) -> Scene {
    let root = protected_root(label);
    let base = root.path().to_path_buf();
    let backups = base.join("backups");
    fs::create_dir(&backups).unwrap_or_else(|error| panic!("{error}"));
    let folder = CanonicalDirectoryPath::new(backups).unwrap_or_else(|error| panic!("{error}"));

    // The archive: another Ledger, backed up through the real pipeline.
    let other_dir = base.join("other");
    fs::create_dir(&other_dir).unwrap_or_else(|error| panic!("{error}"));
    let other_ledger = other_dir.join("product-ledger.sqlite3");
    let mut other = SqliteProductLedger::open(&other_ledger).unwrap_or_else(|e| panic!("{e:?}"));
    add_portfolio(&mut other, "synthetic-archived-a");
    add_portfolio(&mut other, "synthetic-archived-b");
    let other_service = BackupService::in_directory(
        &other_dir,
        "live",
        AuditWorkspace::Live,
        Some(other_ledger.clone()),
    );
    let snapshot = other_service
        .snapshot(&other)
        .unwrap_or_else(|error| panic!("{error:?}"));
    let record = other_service
        .publish(
            &snapshot,
            br#"{"format":"pmc-backup-settings/v1","locale":"ja","timezone":"Asia/Tokyo","theme":"dark","retention_days":60,"external_ai_enabled":false,"log_level":"warn"}"#,
            &folder,
            &passphrase(),
            "a0a0a0a0",
            &|| UtcTimestamp::from_unix_millis(NOW - 2 * HOUR),
            &|path, manifest| pmc_ledger::sqlite::inspect_standalone_snapshot(path, manifest).is_ok(),
        )
        .unwrap_or_else(|error| panic!("{error:?}"));
    let archive = folder.as_path().join(&record.file_name);

    // The live workspace.
    let live_dir = base.join("live");
    fs::create_dir(&live_dir).unwrap_or_else(|error| panic!("{error}"));
    let ledger = live_dir.join("product-ledger.sqlite3");
    let mut live = SqliteProductLedger::open(&ledger).unwrap_or_else(|e| panic!("{e:?}"));
    add_portfolio(&mut live, "synthetic-live-a");
    drop(live);
    let service =
        BackupService::in_directory(&base, "live", AuditWorkspace::Live, Some(ledger.clone()));
    let settings = SettingsStore::open(&root)
        .unwrap_or_else(|error| panic!("{error}"))
        .into_store();
    settings
        .initialize()
        .unwrap_or_else(|error| panic!("{error}"));
    Scene {
        root,
        ledger,
        service,
        settings,
        folder,
        archive,
    }
}

fn record_count(ledger: &Path) -> u64 {
    match inspect_ledger(ledger).unwrap_or_else(|error| panic!("{error:?}")) {
        LedgerCompatibility::Current(inspection) => inspection.inventory.record_count,
        other => panic!("unexpected {other:?}"),
    }
}

/// Check and prepare with the Ledger held, as the host does.
fn prepare(scene: &Scene, intent: &str) -> pmc_application::restore_service::RestorePreview {
    let checked = scene
        .service
        .check_archive(&scene.archive, &passphrase())
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(checked.record_count, 2);
    let settings = scene
        .settings
        .read()
        .unwrap_or_else(|error| panic!("{error}"));
    let live = SqliteProductLedger::open(&scene.ledger).unwrap_or_else(|e| panic!("{e:?}"));
    let preview = scene
        .service
        .prepare_restore(
            &checked,
            &live,
            &settings,
            &RecoveryBackupInput {
                destination: &scene.folder,
                passphrase: &passphrase(),
                archive_id: &format!("{intent}0000").replace('-', ""),
            },
            intent,
            &clock,
            HOUR,
        )
        .unwrap_or_else(|error| panic!("{error:?}"));
    drop(live);
    preview
}

#[test]
fn a_checked_approved_restore_replaces_the_ledger_and_its_settings() {
    let scene = scene("happy");
    let preview = prepare(&scene, "intent-happy");
    assert_eq!(preview.archive_record_count, 2);
    assert_eq!(preview.current_record_count, Some(1));
    // The date to type is the archive's creation date in the current zone.
    let expected_date = local_calendar_date(&preview.archive_created_at, "Asia/Taipei")
        .unwrap_or_else(|| panic!("date"));
    assert_eq!(preview.confirmation_date, expected_date);

    assert!(matches!(
        scene.service.approve_restore(
            "intent-happy",
            &preview.payload_sha256,
            "1999-01-01",
            "execute-1",
            "receipt-1",
            clock(),
        ),
        Err(RestoreError::ConfirmationMismatch)
    ));
    assert!(matches!(
        scene.service.approve_restore(
            "intent-happy",
            &"0".repeat(64),
            &preview.confirmation_date,
            "execute-1",
            "receipt-1",
            clock(),
        ),
        Err(RestoreError::DigestMismatch)
    ));
    assert_eq!(
        scene
            .service
            .approve_restore(
                "intent-happy",
                &preview.payload_sha256,
                &preview.confirmation_date,
                "execute-1",
                "receipt-1",
                clock(),
            )
            .unwrap_or_else(|error| panic!("{error:?}")),
        None
    );
    let outcome = scene
        .service
        .execute_restore(&scene.settings, clock())
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(outcome, RestoreOutcome::Restored);
    assert_eq!(record_count(&scene.ledger), 2);
    let settings = scene
        .settings
        .read()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(settings.display.locale, "ja");
    assert_eq!(settings.display.timezone, "Asia/Tokyo");
    // No rollback copy or staged file is left beside the Ledger.
    let leftovers: Vec<String> = fs::read_dir(scene.ledger.parent().unwrap_or_else(|| panic!()))
        .unwrap_or_else(|error| panic!("{error}"))
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".restore-"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");

    // The same idempotency id returns the outcome it had; nothing runs again.
    assert_eq!(
        scene
            .service
            .approve_restore(
                "intent-happy",
                &preview.payload_sha256,
                &preview.confirmation_date,
                "execute-1",
                "receipt-1",
                clock(),
            )
            .unwrap_or_else(|error| panic!("{error:?}")),
        Some(RestoreOutcome::Restored)
    );
    let _ = &scene.root;
}

#[test]
fn a_ledger_that_changed_after_the_preview_is_not_replaced() {
    let scene = scene("changed");
    let preview = prepare(&scene, "intent-changed");
    scene
        .service
        .approve_restore(
            "intent-changed",
            &preview.payload_sha256,
            &preview.confirmation_date,
            "execute-2",
            "receipt-2",
            clock(),
        )
        .unwrap_or_else(|error| panic!("{error:?}"));
    // A write lands before the host closes the Ledger.
    let mut live = SqliteProductLedger::open(&scene.ledger).unwrap_or_else(|e| panic!("{e:?}"));
    add_portfolio(&mut live, "synthetic-live-late");
    drop(live);
    let outcome = scene
        .service
        .execute_restore(&scene.settings, clock())
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(outcome, RestoreOutcome::FailedBeforeReplacement);
    assert_eq!(record_count(&scene.ledger), 2);
    let settings = scene
        .settings
        .read()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(settings.display.locale, "zh-TW");
}

#[test]
fn a_crash_after_the_ledger_was_moved_aside_is_put_back_at_startup() {
    let scene = scene("crash");
    let preview = prepare(&scene, "intent-crash");
    scene
        .service
        .approve_restore(
            "intent-crash",
            &preview.payload_sha256,
            &preview.confirmation_date,
            "execute-3",
            "receipt-3",
            clock(),
        )
        .unwrap_or_else(|error| panic!("{error:?}"));
    // The process died right after moving the live Ledger aside.
    let previous = scene.ledger.with_file_name(format!(
        "{}.restore-intent-crash.previous",
        scene
            .ledger
            .file_name()
            .unwrap_or_else(|| panic!())
            .to_string_lossy()
    ));
    fs::rename(&scene.ledger, &previous).unwrap_or_else(|error| panic!("{error}"));
    let control = RestoreControlStore::new(scene.root.path().join("restore-control-live-v1.json"));
    control
        .update(|record| -> Result<(), ()> {
            if let Some(RestoreOperation::Executing { phase, .. }) = &mut record.active {
                *phase = RestorePhase::Installing;
            }
            Ok(())
        })
        .unwrap_or_else(|error| panic!("{error}"))
        .unwrap_or_else(|()| panic!("refused"));

    let outcome = scene
        .service
        .reconcile_restore(&scene.settings, clock())
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(outcome, Some(RestoreOutcome::RecoveryPutBack));
    assert!(!scene.service.restore_needs_recovery());
    assert_eq!(record_count(&scene.ledger), 1);
    assert!(!previous.exists());
}

#[test]
fn a_wrong_passphrase_or_a_rejection_changes_nothing() {
    let scene = scene("refused");
    assert!(matches!(
        scene.service.check_archive(
            &scene.archive,
            &Passphrase::new("a different passphrase entirely here".to_owned())
        ),
        Err(RestoreError::WrongPassphrase)
    ));
    let preview = prepare(&scene, "intent-reject");
    scene
        .service
        .reject_restore("intent-reject", clock())
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert!(matches!(
        scene.service.approve_restore(
            "intent-reject",
            &preview.payload_sha256,
            &preview.confirmation_date,
            "execute-4",
            "receipt-4",
            clock(),
        ),
        Err(RestoreError::NotPrepared)
    ));
    assert_eq!(record_count(&scene.ledger), 1);
}

#[test]
fn an_executing_restore_has_one_executor_and_blocks_a_new_check() {
    let scene = scene("claimed");
    let preview = prepare(&scene, "intent-claimed");
    scene
        .service
        .approve_restore(
            "intent-claimed",
            &preview.payload_sha256,
            &preview.confirmation_date,
            "execute-5",
            "receipt-5",
            clock(),
        )
        .unwrap_or_else(|error| panic!("{error:?}"));
    // Executing: the Ledger must not be opened until it is resolved.
    assert!(scene.service.restore_needs_recovery());
    // While it is executing, no other archive may be checked: that would
    // wipe this restore's rollback material.
    assert!(matches!(
        scene.service.check_archive(&scene.archive, &passphrase()),
        Err(RestoreError::AnotherRestoreActive)
    ));
    // The first executor claims it; a second finds nothing to run.
    assert_eq!(
        scene
            .service
            .execute_restore(&scene.settings, clock())
            .unwrap_or_else(|error| panic!("{error:?}")),
        RestoreOutcome::Restored
    );
    assert!(matches!(
        scene.service.execute_restore(&scene.settings, clock()),
        Err(RestoreError::NotPrepared)
    ));
}

#[test]
fn a_picked_file_reopens_and_names_its_recovery_backup() {
    let scene = scene("picked");
    // A folder is not a backup file; the picked archive is, by name only.
    assert!(ChosenArchive::from_picked(scene.folder.as_path().to_path_buf()).is_none());
    let chosen = ChosenArchive::from_picked(scene.archive.clone())
        .unwrap_or_else(|| panic!("the archive is a regular file"));
    assert_eq!(
        chosen.file_name(),
        scene
            .archive
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    );
    let checked = scene
        .service
        .check_chosen_archive(&chosen, &passphrase())
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(checked.record_count, 2);
    assert!(matches!(
        scene.service.reopen_ledger(),
        ReopenedLedger::Ready(_)
    ));
    assert_eq!(scene.service.recovery_archive_needed(), None);

    // Approved but never executed: the Ledger must not open, and System
    // Health names the recovery backup by its file name.
    let preview = prepare(&scene, "intent-picked");
    scene
        .service
        .approve_restore(
            "intent-picked",
            &preview.payload_sha256,
            &preview.confirmation_date,
            "execute-picked",
            "receipt-picked",
            clock(),
        )
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert!(scene.service.restore_needs_recovery());
    let named = scene
        .service
        .recovery_archive_needed()
        .unwrap_or_else(|| panic!("named"));
    assert!(named.starts_with("pmc-operational-"), "{named}");
    assert!(named.contains(&preview.recovery_archive_id), "{named}");
}

#[test]
fn a_settings_change_after_the_preview_stops_the_restore() {
    let scene = scene("settings-changed");
    let preview = prepare(&scene, "intent-settings");
    scene
        .service
        .approve_restore(
            "intent-settings",
            &preview.payload_sha256,
            &preview.confirmation_date,
            "execute-settings",
            "receipt-settings",
            clock(),
        )
        .unwrap_or_else(|error| panic!("{error:?}"));
    // The person changes the language before the restore runs.
    let revision = scene
        .settings
        .read()
        .unwrap_or_else(|error| panic!("{error}"))
        .revision;
    scene
        .settings
        .apply_durable(
            revision,
            SettingsPatch::Display(DisplayPatch {
                locale: Some("ko".to_owned()),
                ..DisplayPatch::default()
            }),
        )
        .unwrap_or_else(|error| panic!("{error}"));
    let outcome = scene
        .service
        .execute_restore(&scene.settings, clock())
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(outcome, RestoreOutcome::FailedBeforeReplacement);
    assert_eq!(record_count(&scene.ledger), 1);
    let settings = scene
        .settings
        .read()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(settings.display.locale, "ko");
}

/// The live Ledger is the frozen v46 fixture — an older supported format, so
/// it is not open and the upgrade gate shows (DG3 restore-unopened amendment,
/// first cut).
fn closed_scene(label: &str) -> Scene {
    let scene = scene(label);
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("pmc-ledger")
        .join("tests")
        .join("fixtures")
        .join("ledger-v46-seeded.sqlite3");
    fs::copy(&fixture, &scene.ledger).unwrap_or_else(|error| panic!("{error}"));
    assert!(matches!(
        inspect_ledger(&scene.ledger).unwrap_or_else(|error| panic!("{error:?}")),
        LedgerCompatibility::UpgradeableFrom(_)
    ));
    scene
}

fn prepare_closed(
    scene: &Scene,
    intent: &str,
) -> Result<pmc_application::restore_service::RestorePreview, RestoreError> {
    let checked = scene
        .service
        .check_archive(&scene.archive, &passphrase())
        .unwrap_or_else(|error| panic!("{error:?}"));
    let settings = scene
        .settings
        .read()
        .unwrap_or_else(|error| panic!("{error}"));
    scene.service.prepare_restore_closed(
        &checked,
        &settings,
        &RecoveryBackupInput {
            destination: &scene.folder,
            passphrase: &passphrase(),
            archive_id: &format!("{intent}0000").replace('-', ""),
        },
        intent,
        &clock,
        HOUR,
    )
}

#[test]
fn a_ledger_waiting_for_its_upgrade_is_backed_up_then_replaced() {
    let scene = closed_scene("closed");
    let LedgerCompatibility::UpgradeableFrom(before) =
        inspect_ledger(&scene.ledger).unwrap_or_else(|error| panic!("{error:?}"))
    else {
        panic!("the fixture is an older supported Ledger");
    };
    let preview =
        prepare_closed(&scene, "intent-closed").unwrap_or_else(|error| panic!("{error:?}"));
    // "Now" is the closed file's own count and last change, read without
    // opening it — never "no change yet" for a Ledger that holds records.
    assert_eq!(preview.current_record_count, Some(139));
    assert!(before.last_change_at_millis.is_some());
    assert_eq!(
        preview.current_last_change_at_millis,
        before.last_change_at_millis
    );
    assert_eq!(preview.archive_record_count, 2);
    // The recovery evidence is an ordinary verified Operational Backup of the
    // closed file, listed like any other.
    let listed = scene
        .service
        .reconcile(Some(&scene.folder))
        .valid_verified_at;
    assert!(
        listed.contains(&preview.recovery_verified_at_millis),
        "{listed:?}"
    );

    scene
        .service
        .approve_restore(
            "intent-closed",
            &preview.payload_sha256,
            &preview.confirmation_date,
            "execute-closed",
            "receipt-closed",
            clock(),
        )
        .unwrap_or_else(|error| panic!("{error:?}"));
    let outcome = scene
        .service
        .execute_restore(&scene.settings, clock())
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(outcome, RestoreOutcome::Restored);
    // The archive's current-version Ledger replaced the v46 file: it opens.
    assert_eq!(record_count(&scene.ledger), 2);
    assert!(matches!(
        scene.service.reopen_ledger(),
        ReopenedLedger::Ready(_)
    ));
    let _ = &scene.root;
}

#[test]
fn a_closed_ledger_that_changed_after_the_preview_is_not_replaced() {
    let scene = closed_scene("closed-changed");
    let preview =
        prepare_closed(&scene, "intent-closed-changed").unwrap_or_else(|error| panic!("{error:?}"));
    scene
        .service
        .approve_restore(
            "intent-closed-changed",
            &preview.payload_sha256,
            &preview.confirmation_date,
            "execute-closed-changed",
            "receipt-closed-changed",
            clock(),
        )
        .unwrap_or_else(|error| panic!("{error:?}"));
    // Another file lands in the Ledger's name before the replacement: a
    // current-version Ledger, so the binding no longer matches.
    let _ = fs::remove_file(&scene.ledger);
    let mut other = SqliteProductLedger::open(&scene.ledger).unwrap_or_else(|e| panic!("{e:?}"));
    add_portfolio(&mut other, "synthetic-swapped-in");
    drop(other);
    let outcome = scene
        .service
        .execute_restore(&scene.settings, clock())
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(outcome, RestoreOutcome::FailedBeforeReplacement);
    assert_eq!(record_count(&scene.ledger), 1);
    let _ = &scene.root;
}

#[test]
fn only_a_ledger_waiting_for_its_upgrade_takes_the_closed_path() {
    // A current Ledger is open in the app: the ordinary path backs it up.
    let scene = scene("closed-refused");
    assert!(matches!(
        prepare_closed(&scene, "intent-closed-refused"),
        Err(RestoreError::CurrentNotInspectable)
    ));
    // Nothing was prepared.
    let control = RestoreControlStore::new(scene.root.path().join("restore-control-live-v1.json"));
    assert!(control
        .load()
        .unwrap_or_else(|error| panic!("{error:?}"))
        .active
        .is_none());
}

// ---- Cut B3: a Ledger PMC cannot inspect, kept as a preservation copy ----

/// The live "Ledger" does not open: garbage bytes, with both sidecars.
fn opaque_scene(label: &str) -> Scene {
    let scene = scene(label);
    fs::write(&scene.ledger, b"not a sqlite file \x00\x01")
        .unwrap_or_else(|error| panic!("{error}"));
    fs::write(sidecar(&scene.ledger, "-wal"), b"wal frames")
        .unwrap_or_else(|error| panic!("{error}"));
    fs::write(sidecar(&scene.ledger, "-shm"), vec![7u8; 4096])
        .unwrap_or_else(|error| panic!("{error}"));
    scene
}

fn sidecar(ledger: &Path, suffix: &str) -> PathBuf {
    let mut name = ledger.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn file_set(ledger: &Path) -> Vec<Option<Vec<u8>>> {
    ["", "-wal", "-shm"]
        .iter()
        .map(|suffix| fs::read(sidecar(ledger, suffix)).ok())
        .collect()
}

fn prepare_preserved(
    scene: &Scene,
    intent: &str,
    reason: UnopenedReason,
) -> pmc_application::restore_service::RestorePreview {
    let checked = scene
        .service
        .check_archive(&scene.archive, &passphrase())
        .unwrap_or_else(|error| panic!("{error:?}"));
    let settings = scene
        .settings
        .read()
        .unwrap_or_else(|error| panic!("{error}"));
    scene
        .service
        .prepare_restore_preserved(
            &checked,
            &settings,
            reason,
            &RecoveryBackupInput {
                destination: &scene.folder,
                passphrase: &passphrase(),
                archive_id: &format!("{intent}0000").replace('-', ""),
            },
            intent,
            &clock,
            HOUR,
        )
        .unwrap_or_else(|error| panic!("{error:?}"))
}

fn approve(
    scene: &Scene,
    intent: &str,
    preview: &pmc_application::restore_service::RestorePreview,
) {
    scene
        .service
        .approve_restore(
            intent,
            &preview.payload_sha256,
            &preview.confirmation_date,
            &format!("execute-{intent}"),
            &format!("receipt-{intent}"),
            clock(),
        )
        .unwrap_or_else(|error| panic!("{error:?}"));
}

fn control(scene: &Scene) -> RestoreControlStore {
    RestoreControlStore::new(scene.root.path().join("restore-control-live-v1.json"))
}

#[test]
fn a_ledger_that_does_not_open_is_preserved_then_replaced() {
    let scene = opaque_scene("opaque");
    let before = file_set(&scene.ledger);
    let preview = prepare_preserved(&scene, "intent-opaque", UnopenedReason::OpenFailed);
    // Nothing invented about a Ledger PMC cannot read.
    assert_eq!(preview.current_record_count, None);
    assert_eq!(preview.current_last_change_at_millis, None);
    assert_eq!(preview.recovery_kind, RecoveryKind::PreservationCopy);
    assert!(
        preview.recovery_name.starts_with("pmc-preservation-"),
        "{}",
        preview.recovery_name
    );
    // The copy is in the backup folder, and not an Operational Backup.
    assert!(scene
        .folder
        .as_path()
        .join(&preview.recovery_name)
        .is_file());
    assert!(!scene
        .service
        .reconcile(Some(&scene.folder))
        .valid_verified_at
        .contains(&preview.recovery_verified_at_millis));

    approve(&scene, "intent-opaque", &preview);
    let outcome = scene
        .service
        .execute_restore(&scene.settings, clock())
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(outcome, RestoreOutcome::Restored);
    assert_eq!(record_count(&scene.ledger), 2);
    // No rollback names left beside the Ledger; the copy stays in the folder.
    let leftovers: Vec<String> = fs::read_dir(scene.ledger.parent().unwrap_or_else(|| panic!()))
        .unwrap_or_else(|error| panic!("{error}"))
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".restore-"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    assert!(scene
        .folder
        .as_path()
        .join(&preview.recovery_name)
        .is_file());
    assert_ne!(file_set(&scene.ledger), before);
}

#[test]
fn a_restore_over_an_open_ledger_leaves_other_rollback_names_alone() {
    // The inspectable path never moves sidecars, so it never removes a file
    // at a sidecar's rollback name.
    let scene = scene("inspectable-cleanup");
    let preview = prepare(&scene, "intent-cleanup");
    let stranger = sidecar(&scene.ledger, ".restore-intent-cleanup.previous-wal");
    fs::write(&stranger, b"not ours to remove").unwrap_or_else(|error| panic!("{error}"));
    approve(&scene, "intent-cleanup", &preview);
    assert_eq!(
        scene
            .service
            .execute_restore(&scene.settings, clock())
            .unwrap_or_else(|error| panic!("{error:?}")),
        RestoreOutcome::Restored
    );
    scene
        .service
        .reconcile_restore(&scene.settings, clock())
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(
        fs::read(&stranger).unwrap_or_else(|error| panic!("{error}")),
        b"not ours to remove"
    );
}

#[test]
fn a_changed_file_set_is_not_replaced() {
    let scene = opaque_scene("opaque-changed");
    let preview = prepare_preserved(&scene, "intent-opaque-changed", UnopenedReason::OpenFailed);
    approve(&scene, "intent-opaque-changed", &preview);
    // A sidecar changes after the preview.
    fs::write(sidecar(&scene.ledger, "-wal"), b"other frames")
        .unwrap_or_else(|error| panic!("{error}"));
    let after_change = file_set(&scene.ledger);
    let outcome = scene
        .service
        .execute_restore(&scene.settings, clock())
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(outcome, RestoreOutcome::FailedBeforeReplacement);
    assert_eq!(file_set(&scene.ledger), after_change);
}

#[cfg(windows)]
#[test]
fn a_file_another_program_holds_is_not_replaced() {
    use std::os::windows::fs::OpenOptionsExt;
    let scene = opaque_scene("opaque-locked");
    let before = file_set(&scene.ledger);
    let preview = prepare_preserved(&scene, "intent-opaque-locked", UnopenedReason::OpenFailed);
    approve(&scene, "intent-opaque-locked", &preview);
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(sidecar(&scene.ledger, "-shm"))
        .unwrap_or_else(|error| panic!("{error}"));
    let outcome = scene
        .service
        .execute_restore(&scene.settings, clock())
        .unwrap_or_else(|error| panic!("{error:?}"));
    drop(held);
    assert_eq!(outcome, RestoreOutcome::FailedBeforeReplacement);
    assert_eq!(file_set(&scene.ledger), before);
}

/// An executing restore interrupted at `phase` after the Ledger and the
/// sidecars in `moved` went to their rollback names, with a copy installed
/// when `installed`: what a crash at that point leaves.
fn interrupted(
    scene: &Scene,
    intent: &str,
    phase: RestorePhase,
    moved: &[&str],
    installed: bool,
) -> Vec<Option<Vec<u8>>> {
    let before = file_set(&scene.ledger);
    let preview = prepare_preserved(scene, intent, UnopenedReason::OpenFailed);
    approve(scene, intent, &preview);
    control(scene)
        .update(|control| match &mut control.active {
            Some(RestoreOperation::Executing { phase: current, .. }) => {
                *current = phase;
                Ok(())
            }
            _ => Err(()),
        })
        .unwrap_or_else(|error| panic!("{error}"))
        .unwrap_or_else(|()| panic!("not executing"));
    let previous = sidecar(&scene.ledger, &format!(".restore-{intent}.previous"));
    fs::rename(&scene.ledger, &previous).unwrap_or_else(|error| panic!("{error}"));
    for suffix in moved {
        fs::rename(sidecar(&scene.ledger, suffix), sidecar(&previous, suffix))
            .unwrap_or_else(|error| panic!("{error}"));
    }
    if installed {
        // The installed copy, and a sidecar of its own at the live name.
        fs::write(&scene.ledger, b"installed copy").unwrap_or_else(|error| panic!("{error}"));
        fs::write(sidecar(&scene.ledger, "-wal"), b"installed frames")
            .unwrap_or_else(|error| panic!("{error}"));
    }
    before
}

fn interrupted_after_moving(scene: &Scene, intent: &str) -> Vec<Option<Vec<u8>>> {
    interrupted(scene, intent, RestorePhase::Installing, &["-wal"], true)
}

#[test]
fn a_crash_at_any_point_after_the_first_move_puts_every_file_back() {
    for (index, (phase, moved, installed)) in [
        (RestorePhase::MovingLiveAside, &[][..], false),
        (RestorePhase::MovingLiveAside, &["-wal"][..], false),
        (RestorePhase::MovingLiveAside, &["-wal", "-shm"][..], false),
        (RestorePhase::Installing, &["-wal", "-shm"][..], true),
        (RestorePhase::ApplyingSettings, &["-wal", "-shm"][..], true),
        (RestorePhase::Verifying, &["-wal", "-shm"][..], true),
    ]
    .into_iter()
    .enumerate()
    {
        let scene = opaque_scene(&format!("opaque-point-{index}"));
        let intent = format!("intent-point-{index}");
        let before = interrupted(&scene, &intent, phase, moved, installed);
        assert_eq!(
            scene
                .service
                .reconcile_restore(&scene.settings, clock())
                .unwrap_or_else(|error| panic!("{error:?}")),
            Some(RestoreOutcome::RecoveryPutBack),
            "{phase:?} {moved:?}"
        );
        assert_eq!(file_set(&scene.ledger), before, "{phase:?} {moved:?}");
        // Nothing left at a rollback name.
        let leftovers: Vec<String> =
            fs::read_dir(scene.ledger.parent().unwrap_or_else(|| panic!()))
                .unwrap_or_else(|error| panic!("{error}"))
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.contains(".restore-"))
                .collect();
        assert!(leftovers.is_empty(), "{phase:?} {moved:?}: {leftovers:?}");
    }
}

#[test]
fn a_crash_mid_move_puts_every_file_back_exactly() {
    let scene = opaque_scene("opaque-crash");
    let before = interrupted_after_moving(&scene, "intent-opaque-crash");
    let outcome = scene
        .service
        .reconcile_restore(&scene.settings, clock())
        .unwrap_or_else(|error| panic!("{error:?}"));
    // Put back — and only claimed because the files match what was bound;
    // the `-shm` that never moved stayed where it was.
    assert_eq!(outcome, Some(RestoreOutcome::RecoveryPutBack));
    assert_eq!(file_set(&scene.ledger), before);
    assert!(!scene.service.restore_needs_recovery());
}

#[test]
fn a_failed_recovery_stays_failed_until_a_later_restore_resolves_it() {
    let scene = opaque_scene("opaque-recovery");
    interrupted_after_moving(&scene, "intent-opaque-lost");
    // The rollback copy of the Ledger is gone: it cannot be put back.
    fs::remove_file(sidecar(
        &scene.ledger,
        ".restore-intent-opaque-lost.previous",
    ))
    .unwrap_or_else(|error| panic!("{error}"));
    let outcome = scene
        .service
        .reconcile_restore(&scene.settings, clock())
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(outcome, Some(RestoreOutcome::RecoveryFailed));
    assert!(scene.service.restore_needs_recovery());
    // Never forward: a second start changes nothing.
    assert_eq!(
        scene
            .service
            .reconcile_restore(&scene.settings, clock())
            .unwrap_or_else(|error| panic!("{error:?}")),
        None
    );
    assert!(scene.service.restore_needs_recovery());

    // A restore over the uncertain files resolves it, and keeps the record.
    let preview = prepare_preserved(
        &scene,
        "intent-opaque-resolve",
        UnopenedReason::RestoreRecoveryRequired,
    );
    approve(&scene, "intent-opaque-resolve", &preview);
    assert_eq!(
        scene
            .service
            .execute_restore(&scene.settings, clock())
            .unwrap_or_else(|error| panic!("{error:?}")),
        RestoreOutcome::Restored
    );
    assert!(!scene.service.restore_needs_recovery());
    let finished = control(&scene)
        .load()
        .unwrap_or_else(|error| panic!("{error}"))
        .finished;
    assert_eq!(finished.len(), 2);
    assert_eq!(finished[0].outcome, RestoreOutcome::RecoveryFailed);
    assert_eq!(
        finished[1].resolved_recovery_failure.as_deref(),
        Some(finished[0].idempotency_id.as_str())
    );
}
