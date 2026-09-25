use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Barrier;
use std::time::{SystemTime, UNIX_EPOCH};

use atomic_write_file::AtomicWriteFile;
use pmc_platform::settings::{
    startup_selection, BackupSettings, CancelOutcome, CanonicalDirectoryPath,
    DeferredDirectoryPath, DisplayPatch, DisplaySettings, FlushOutcome, InvalidSettingsReason,
    LoadDisposition, OperationalPatch, OperationalSettings, PatchOutcome, PresentationPatch,
    PresentationSettings, ProtectedSettingsRoot, QueueOutcome, SelectedWorkspace, SettingsDocument,
    SettingsPatch, SettingsStore, StartupSelection, TableDensity, TablePreference, Theme,
    ValuePatch, WindowGeometry, FOLLOW_SYSTEM_LOCALE,
};

const INTERRUPTION_HELPER_ROOT: &str = "PMC_SETTINGS_INTERRUPTION_HELPER_ROOT";
static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Some Windows dev/CI sandboxes (this repo's own included) run the test
/// process inside an app-package container that transparently redirects
/// writes under the real `%LOCALAPPDATA%` to a per-package storage area
/// (`...\AppData\Local\Packages\<container>\LocalCache\Local\...`).
/// `ProtectedSettingsRoot::prepare`'s `canonical == path` check (a real
/// defense against a directory materializing somewhere unexpected) then
/// legitimately fails, because it *did* materialize somewhere unexpected --
/// just for a benign OS reason unrelated to what the check defends against.
/// Production behavior (`os_app_data_base`) is untouched: this only points
/// the *test process's* `%LOCALAPPDATA%` at a plain directory under this
/// crate's own `target/`, which this repo's sandbox does not redirect, so
/// every test still exercises the exact same `ProtectedSettingsRoot`
/// codepath end to end, just rooted somewhere the container leaves alone.
#[cfg(windows)]
fn ensure_test_app_data_root_is_not_redirected() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("pmc-platform-test-app-data");
        std::fs::create_dir_all(&root)
            .unwrap_or_else(|error| panic!("test app-data override root setup failed: {error}"));
        let root = std::fs::canonicalize(&root)
            .unwrap_or_else(|error| panic!("test app-data override root setup failed: {error}"));
        // SAFETY: this runs once, guarded by `Once`, before any test spawns
        // additional threads or reads `LOCALAPPDATA`; `Once::call_once`
        // establishes a happens-before edge for every later caller.
        unsafe {
            std::env::set_var("LOCALAPPDATA", root);
        }
    });
}

#[cfg(not(windows))]
fn ensure_test_app_data_root_is_not_redirected() {}

fn unique_application_directory(test_name: &str) -> String {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("product-mission-control-test-{test_name}-{nonce}-{sequence}")
}

fn isolated_settings_root(test_name: &str) -> ProtectedSettingsRoot {
    ensure_test_app_data_root_is_not_redirected();
    ProtectedSettingsRoot::prepare(&unique_application_directory(test_name))
        .unwrap_or_else(|error| panic!("protected root setup failed: {error}"))
}

#[test]
fn queued_presentation_patch_rebases_over_a_newer_operational_commit() {
    let root = isolated_settings_root("settings-rebase");
    let opened = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    opened
        .store()
        .initialize()
        .unwrap_or_else(|error| panic!("initial commit failed: {error}"));

    assert_eq!(
        opened
            .store()
            .queue_presentation(
                0,
                PresentationPatch {
                    last_route: ValuePatch::Set("/executive-cockpit".to_owned()),
                    sidebar_width: ValuePatch::Set(320),
                    ..PresentationPatch::default()
                },
            )
            .unwrap_or_else(|error| panic!("queue failed: {error}")),
        QueueOutcome::Queued { base_revision: 0 }
    );

    assert_eq!(
        opened
            .store()
            .apply_durable(
                0,
                SettingsPatch::Operational(OperationalPatch {
                    retention_days: Some(120),
                    ..OperationalPatch::default()
                }),
            )
            .unwrap_or_else(|error| panic!("operational commit failed: {error}")),
        PatchOutcome::Committed { revision: 1 }
    );
    assert_eq!(
        opened
            .store()
            .flush_pending_presentation()
            .unwrap_or_else(|error| panic!("flush failed: {error}")),
        FlushOutcome::Committed { revision: 2 }
    );

    let reopened =
        SettingsStore::open(&root).unwrap_or_else(|error| panic!("reopen failed: {error}"));
    assert_eq!(reopened.document().revision, 2);
    assert_eq!(reopened.document().operational.retention_days, 120);
    assert_eq!(
        reopened.document().presentation.last_route.as_deref(),
        Some("/executive-cockpit")
    );
    assert_eq!(reopened.document().presentation.sidebar_width, Some(320));

    drop(reopened);
    drop(opened);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn stale_presentation_patch_cannot_roll_back_a_newer_operational_commit() {
    let root = isolated_settings_root("settings-cas");
    let opened = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    opened
        .store()
        .initialize()
        .unwrap_or_else(|error| panic!("initial commit failed: {error}"));

    let operational = opened
        .store()
        .apply_durable(
            0,
            SettingsPatch::Operational(OperationalPatch {
                retention_days: Some(90),
                ..OperationalPatch::default()
            }),
        )
        .unwrap_or_else(|error| panic!("operational patch failed: {error}"));
    assert_eq!(operational, PatchOutcome::Committed { revision: 1 });

    let stale_presentation = opened
        .store()
        .apply_durable(
            0,
            SettingsPatch::Presentation(PresentationPatch {
                last_route: ValuePatch::Set("/weekly-review".to_owned()),
                ..PresentationPatch::default()
            }),
        )
        .unwrap_or_else(|error| panic!("presentation patch failed: {error}"));
    assert_eq!(
        stale_presentation,
        PatchOutcome::Stale {
            current_revision: 1
        }
    );

    let reopened =
        SettingsStore::open(&root).unwrap_or_else(|error| panic!("reopen failed: {error}"));
    assert_eq!(reopened.document().operational.retention_days, 90);
    assert_eq!(reopened.document().presentation.last_route, None);
    assert_eq!(reopened.document().revision, 1);

    drop(reopened);
    drop(opened);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn defaults_commit_then_close_and_reopen_round_trips_the_versioned_document() {
    let root = isolated_settings_root("settings-reopen");

    let first_open =
        SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    assert_eq!(first_open.disposition(), LoadDisposition::MissingDefaulted);
    assert_eq!(first_open.document(), &SettingsDocument::default());

    let expected = SettingsDocument {
        format_version: 3,
        revision: 3,
        display: DisplaySettings {
            locale: "zh-TW".to_owned(),
            timezone: "Asia/Taipei".to_owned(),
            ..DisplaySettings::default()
        },
        operational: OperationalSettings {
            retention_days: 45,
            ..OperationalSettings::default()
        },
        presentation: PresentationSettings {
            last_route: Some("/work-queue".to_owned()),
            sidebar_width: Some(288),
            window: Some(WindowGeometry {
                x: 40,
                y: 60,
                width: 1440,
                height: 900,
                maximized: false,
            }),
            table_preferences: vec![TablePreference {
                table_id: "synthetic-work-queue".to_owned(),
                density: TableDensity::Compact,
                visible_columns: vec!["title".to_owned(), "status".to_owned()],
            }],
        },
    };

    first_open
        .store()
        .initialize()
        .unwrap_or_else(|error| panic!("initialize failed: {error}"));
    first_open
        .store()
        .apply_durable(
            0,
            SettingsPatch::Display(DisplayPatch {
                locale: Some(expected.display.locale.clone()),
                timezone: Some(expected.display.timezone.clone()),
                theme: Some(expected.display.theme),
            }),
        )
        .unwrap_or_else(|error| panic!("display commit failed: {error}"));
    first_open
        .store()
        .apply_durable(
            1,
            SettingsPatch::Operational(OperationalPatch {
                retention_days: Some(expected.operational.retention_days),
                ..OperationalPatch::default()
            }),
        )
        .unwrap_or_else(|error| panic!("operational commit failed: {error}"));
    first_open
        .store()
        .apply_durable(
            2,
            SettingsPatch::Presentation(PresentationPatch {
                last_route: ValuePatch::Set(
                    expected
                        .presentation
                        .last_route
                        .clone()
                        .unwrap_or_else(|| panic!("missing expected route")),
                ),
                sidebar_width: ValuePatch::Set(
                    expected
                        .presentation
                        .sidebar_width
                        .unwrap_or_else(|| panic!("missing expected sidebar width")),
                ),
                window: ValuePatch::Set(
                    expected
                        .presentation
                        .window
                        .clone()
                        .unwrap_or_else(|| panic!("missing expected window")),
                ),
                table_preferences: Some(expected.presentation.table_preferences.clone()),
            }),
        )
        .unwrap_or_else(|error| panic!("presentation commit failed: {error}"));
    drop(first_open);

    let reopened =
        SettingsStore::open(&root).unwrap_or_else(|error| panic!("reopen failed: {error}"));
    assert_eq!(reopened.disposition(), LoadDisposition::Loaded);
    assert_eq!(reopened.document(), &expected);

    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn invalid_file_is_preserved_and_open_uses_safe_defaults() {
    let root = isolated_settings_root("settings-invalid");
    fs::create_dir_all(&root).unwrap_or_else(|error| panic!("setup failed: {error}"));
    let settings_path = root.join("settings-v1.json");
    let invalid_bytes = br#"{"#;
    fs::write(&settings_path, invalid_bytes)
        .unwrap_or_else(|error| panic!("fixture write failed: {error}"));

    let opened = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    let preserved_path = match opened.disposition() {
        LoadDisposition::InvalidPreserved { path, reason } => {
            assert_eq!(reason, InvalidSettingsReason::Malformed);
            path
        }
        disposition => panic!("unexpected load disposition: {disposition:?}"),
    };

    assert_eq!(opened.document(), &SettingsDocument::default());
    assert!(!settings_path.exists());
    assert_eq!(
        fs::read(&preserved_path).unwrap_or_else(|error| panic!("preserved read failed: {error}")),
        invalid_bytes
    );
    let canonical_root =
        fs::canonicalize(&root).unwrap_or_else(|error| panic!("canonicalize failed: {error}"));
    assert!(preserved_path.starts_with(canonical_root.join("diagnostics")));

    drop(opened);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn concurrent_writers_are_serialized_and_only_one_matching_revision_commits() {
    let root = isolated_settings_root("settings-concurrent");
    let opened = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    opened
        .store()
        .initialize()
        .unwrap_or_else(|error| panic!("initial commit failed: {error}"));
    let barrier = Barrier::new(3);

    let outcomes = std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            barrier.wait();
            opened.store().apply_durable(
                0,
                SettingsPatch::Operational(OperationalPatch {
                    retention_days: Some(60),
                    ..OperationalPatch::default()
                }),
            )
        });
        let second = scope.spawn(|| {
            barrier.wait();
            opened.store().apply_durable(
                0,
                SettingsPatch::Operational(OperationalPatch {
                    retention_days: Some(120),
                    ..OperationalPatch::default()
                }),
            )
        });
        barrier.wait();
        [
            first
                .join()
                .unwrap_or_else(|_| panic!("first writer panicked"))
                .unwrap_or_else(|error| panic!("first writer failed: {error}")),
            second
                .join()
                .unwrap_or_else(|_| panic!("second writer panicked"))
                .unwrap_or_else(|error| panic!("second writer failed: {error}")),
        ]
    });

    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, PatchOutcome::Committed { revision: 1 }))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(
                outcome,
                PatchOutcome::Stale {
                    current_revision: 1
                }
            ))
            .count(),
        1
    );

    let reopened =
        SettingsStore::open(&root).unwrap_or_else(|error| panic!("reopen failed: {error}"));
    assert_eq!(reopened.document().revision, 1);
    assert!([60, 120].contains(&reopened.document().operational.retention_days));

    drop(reopened);
    drop(opened);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn presentation_queue_coalesces_fields_and_keeps_the_latest_value() {
    let root = isolated_settings_root("settings-coalesce");
    let opened = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    opened
        .store()
        .initialize()
        .unwrap_or_else(|error| panic!("initial commit failed: {error}"));

    opened
        .store()
        .queue_presentation(
            0,
            PresentationPatch {
                last_route: ValuePatch::Set("/work-queue".to_owned()),
                sidebar_width: ValuePatch::Set(280),
                ..PresentationPatch::default()
            },
        )
        .unwrap_or_else(|error| panic!("first queue failed: {error}"));
    opened
        .store()
        .queue_presentation(
            0,
            PresentationPatch {
                last_route: ValuePatch::Set("/weekly-review".to_owned()),
                ..PresentationPatch::default()
            },
        )
        .unwrap_or_else(|error| panic!("second queue failed: {error}"));
    assert_eq!(
        opened
            .store()
            .flush_pending_presentation()
            .unwrap_or_else(|error| panic!("flush failed: {error}")),
        FlushOutcome::Committed { revision: 1 }
    );

    let reopened =
        SettingsStore::open(&root).unwrap_or_else(|error| panic!("reopen failed: {error}"));
    assert_eq!(
        reopened.document().presentation.last_route.as_deref(),
        Some("/weekly-review")
    );
    assert_eq!(reopened.document().presentation.sidebar_width, Some(280));

    drop(reopened);
    drop(opened);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn dropping_an_uncommitted_atomic_write_preserves_the_canonical_document() {
    let root = isolated_settings_root("settings-interruption");
    let opened = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    let expected = SettingsDocument::default();
    opened
        .store()
        .initialize()
        .unwrap_or_else(|error| panic!("initial commit failed: {error}"));
    let canonical = root.join("settings-v1.json");

    let mut interrupted = AtomicWriteFile::open(&canonical)
        .unwrap_or_else(|error| panic!("atomic open failed: {error}"));
    interrupted
        .write_all(br#"{"format_version":1"#)
        .unwrap_or_else(|error| panic!("partial write failed: {error}"));
    drop(interrupted);

    let reopened =
        SettingsStore::open(&root).unwrap_or_else(|error| panic!("reopen failed: {error}"));
    assert_eq!(reopened.disposition(), LoadDisposition::Loaded);
    assert_eq!(reopened.document(), &expected);

    drop(reopened);
    drop(opened);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn settings_schema_rejects_secret_fields_without_echoing_the_value() {
    let root = isolated_settings_root("settings-secret-rejection");
    fs::create_dir_all(&root).unwrap_or_else(|error| panic!("setup failed: {error}"));
    let settings_path = root.join("settings-v1.json");
    let secret_value = "synthetic-secret-value-should-never-appear";
    let document = format!(
        r#"{{"format_version":1,"revision":0,"display":{{"locale":"zh-TW","timezone":"Asia/Taipei","theme":"system"}},"operational":{{"authority_root":null,"backup_destination":null,"live_vault_root":null,"retention_days":30,"external_ai_enabled":false,"log_level":"info"}},"presentation":{{"last_route":null,"sidebar_width":null,"window":null,"table_preferences":[]}},"provider_secret":"{secret_value}"}}"#
    );
    fs::write(&settings_path, document)
        .unwrap_or_else(|error| panic!("fixture write failed: {error}"));

    let opened = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    assert!(matches!(
        opened.disposition(),
        LoadDisposition::InvalidPreserved {
            reason: InvalidSettingsReason::Schema,
            ..
        }
    ));
    assert_eq!(opened.document(), &SettingsDocument::default());

    let serialized = serde_json::to_string(opened.document())
        .unwrap_or_else(|error| panic!("serialization failed: {error}"));
    assert!(!serialized.contains(secret_value));
    assert!(!serialized.contains("provider_secret"));

    drop(opened);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn typed_group_patches_preserve_unrelated_groups_and_can_clear_optional_paths() {
    let root = isolated_settings_root("settings-typed-patches");
    let opened = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    fs::create_dir(root.join("live-authority"))
        .unwrap_or_else(|error| panic!("authority fixture failed: {error}"));
    fs::create_dir(root.join("backup"))
        .unwrap_or_else(|error| panic!("backup fixture failed: {error}"));
    let authority_root = CanonicalDirectoryPath::new(root.join("live-authority"))
        .unwrap_or_else(|error| panic!("authority path failed: {error}"));
    let backup_destination = CanonicalDirectoryPath::new(root.join("backup"))
        .unwrap_or_else(|error| panic!("backup path failed: {error}"));
    let last_route = "/work-queue".to_owned();
    opened
        .store()
        .initialize()
        .unwrap_or_else(|error| panic!("initial commit failed: {error}"));
    opened
        .store()
        .apply_durable(
            0,
            SettingsPatch::Operational(OperationalPatch {
                authority_root: ValuePatch::Set(authority_root.clone()),
                backup_destination: ValuePatch::Set(backup_destination.into()),
                ..OperationalPatch::default()
            }),
        )
        .unwrap_or_else(|error| panic!("initial operational patch failed: {error}"));
    opened
        .store()
        .apply_durable(
            1,
            SettingsPatch::Presentation(PresentationPatch {
                last_route: ValuePatch::Set(last_route.clone()),
                ..PresentationPatch::default()
            }),
        )
        .unwrap_or_else(|error| panic!("initial presentation patch failed: {error}"));

    assert_eq!(
        opened
            .store()
            .apply_durable(
                2,
                SettingsPatch::Display(DisplayPatch {
                    locale: Some("en-US".to_owned()),
                    theme: Some(Theme::Dark),
                    ..DisplayPatch::default()
                }),
            )
            .unwrap_or_else(|error| panic!("display patch failed: {error}")),
        PatchOutcome::Committed { revision: 3 }
    );
    assert_eq!(
        opened
            .store()
            .apply_durable(
                3,
                SettingsPatch::Operational(OperationalPatch {
                    backup_destination: ValuePatch::Clear,
                    ..OperationalPatch::default()
                }),
            )
            .unwrap_or_else(|error| panic!("operational patch failed: {error}")),
        PatchOutcome::Committed { revision: 4 }
    );

    let reopened =
        SettingsStore::open(&root).unwrap_or_else(|error| panic!("reopen failed: {error}"));
    assert_eq!(reopened.document().display.locale, "en-US");
    assert_eq!(reopened.document().display.theme, Theme::Dark);
    assert_eq!(
        reopened.document().operational.authority_root,
        Some(authority_root)
    );
    assert_eq!(reopened.document().operational.backup_destination, None);
    assert_eq!(
        reopened.document().presentation.last_route,
        Some(last_route)
    );

    drop(reopened);
    drop(opened);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn initialization_is_default_only_and_cannot_overwrite_authority() {
    let root = isolated_settings_root("settings-initial-overwrite");
    let opened = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    opened
        .store()
        .initialize()
        .unwrap_or_else(|error| panic!("initial commit failed: {error}"));
    opened
        .store()
        .apply_durable(
            0,
            SettingsPatch::Operational(OperationalPatch {
                retention_days: Some(90),
                ..OperationalPatch::default()
            }),
        )
        .unwrap_or_else(|error| panic!("operational patch failed: {error}"));
    assert_eq!(
        opened
            .store()
            .initialize()
            .unwrap_or_else(|error| panic!("second initialize failed: {error}")),
        pmc_platform::settings::InitializeOutcome::AlreadyInitialized {
            current_revision: 1
        }
    );

    let reopened =
        SettingsStore::open(&root).unwrap_or_else(|error| panic!("reopen failed: {error}"));
    assert_eq!(reopened.document().revision, 1);
    assert_eq!(reopened.document().operational.retention_days, 90);
    assert!(!reopened.document().operational.external_ai_enabled);

    drop(reopened);
    drop(opened);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn first_run_initialization_can_choose_the_locale_in_one_write() {
    let root = isolated_settings_root("settings-initial-locale");
    let opened = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    assert_eq!(
        opened
            .store()
            .initialize_with_locale(FOLLOW_SYSTEM_LOCALE)
            .unwrap_or_else(|error| panic!("initialize failed: {error}")),
        pmc_platform::settings::InitializeOutcome::Initialized { revision: 0 }
    );
    // A second first-run cannot overwrite what is already there.
    assert_eq!(
        opened
            .store()
            .initialize_with_locale("zh-TW")
            .unwrap_or_else(|error| panic!("second initialize failed: {error}")),
        pmc_platform::settings::InitializeOutcome::AlreadyInitialized {
            current_revision: 0
        }
    );

    let reopened =
        SettingsStore::open(&root).unwrap_or_else(|error| panic!("reopen failed: {error}"));
    assert_eq!(reopened.disposition(), LoadDisposition::Loaded);
    let expected = SettingsDocument {
        display: DisplaySettings {
            locale: "und".to_owned(),
            ..DisplaySettings::default()
        },
        ..SettingsDocument::default()
    };
    assert_eq!(reopened.document(), &expected);
    assert_eq!(
        reopened
            .store()
            .read()
            .unwrap_or_else(|error| panic!("read failed: {error}")),
        expected
    );

    drop(reopened);
    drop(opened);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn first_run_initialization_refuses_an_invalid_locale_without_writing() {
    let root = isolated_settings_root("settings-initial-bad-locale");
    let opened = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    assert!(opened
        .store()
        .initialize_with_locale("not a locale!")
        .is_err());
    let reopened =
        SettingsStore::open(&root).unwrap_or_else(|error| panic!("reopen failed: {error}"));
    assert_eq!(reopened.disposition(), LoadDisposition::MissingDefaulted);

    drop(reopened);
    drop(opened);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn an_opened_store_can_be_kept_and_still_writes() {
    let root = isolated_settings_root("settings-into-store");
    let store = SettingsStore::open(&root)
        .unwrap_or_else(|error| panic!("open failed: {error}"))
        .into_store();
    store
        .initialize_with_locale("en")
        .unwrap_or_else(|error| panic!("initialize failed: {error}"));
    assert_eq!(
        store
            .apply_durable(
                0,
                SettingsPatch::Display(DisplayPatch {
                    locale: Some("ja".to_owned()),
                    ..DisplayPatch::default()
                }),
            )
            .unwrap_or_else(|error| panic!("patch failed: {error}")),
        PatchOutcome::Committed { revision: 1 }
    );
    assert_eq!(
        store
            .read()
            .unwrap_or_else(|error| panic!("read failed: {error}"))
            .display
            .locale,
        "ja"
    );

    drop(store);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn unsupported_format_version_is_preserved_with_a_structured_migration_result() {
    let root = isolated_settings_root("settings-unsupported-version");
    fs::create_dir_all(&root).unwrap_or_else(|error| panic!("setup failed: {error}"));
    fs::write(
        root.join("settings-v1.json"),
        br#"{"format_version":4,"future_shape":true}"#,
    )
    .unwrap_or_else(|error| panic!("fixture write failed: {error}"));

    let opened = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    assert!(matches!(
        opened.disposition(),
        LoadDisposition::InvalidPreserved {
            reason: InvalidSettingsReason::UnsupportedFormatVersion {
                found: 4,
                supported: 3
            },
            ..
        }
    ));
    assert_eq!(opened.document(), &SettingsDocument::default());

    drop(opened);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn independent_open_handles_share_one_cas_writer() {
    let root = isolated_settings_root("settings-independent-handles");
    let initializer =
        SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    initializer
        .store()
        .initialize()
        .unwrap_or_else(|error| panic!("initial commit failed: {error}"));
    let first = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    let second = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    let barrier = Barrier::new(3);

    let outcomes = std::thread::scope(|scope| {
        let first_writer = scope.spawn(|| {
            barrier.wait();
            first.store().apply_durable(
                0,
                SettingsPatch::Operational(OperationalPatch {
                    retention_days: Some(60),
                    ..OperationalPatch::default()
                }),
            )
        });
        let second_writer = scope.spawn(|| {
            barrier.wait();
            second.store().apply_durable(
                0,
                SettingsPatch::Operational(OperationalPatch {
                    retention_days: Some(120),
                    ..OperationalPatch::default()
                }),
            )
        });
        barrier.wait();
        [
            first_writer
                .join()
                .unwrap_or_else(|_| panic!("first writer panicked"))
                .unwrap_or_else(|error| panic!("first writer failed: {error}")),
            second_writer
                .join()
                .unwrap_or_else(|_| panic!("second writer panicked"))
                .unwrap_or_else(|error| panic!("second writer failed: {error}")),
        ]
    });

    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, PatchOutcome::Committed { revision: 1 }))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(
                outcome,
                PatchOutcome::Stale {
                    current_revision: 1
                }
            ))
            .count(),
        1
    );
    let reopened =
        SettingsStore::open(&root).unwrap_or_else(|error| panic!("reopen failed: {error}"));
    assert_eq!(reopened.document().revision, 1);

    drop(reopened);
    drop(second);
    drop(first);
    drop(initializer);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn invalid_patch_is_rejected_before_it_can_become_authoritative() {
    let root = isolated_settings_root("settings-invalid-patch");
    let opened = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    opened
        .store()
        .initialize()
        .unwrap_or_else(|error| panic!("initial commit failed: {error}"));

    assert!(opened
        .store()
        .apply_durable(
            0,
            SettingsPatch::Operational(OperationalPatch {
                retention_days: Some(0),
                ..OperationalPatch::default()
            }),
        )
        .is_err());
    assert!(CanonicalDirectoryPath::new(PathBuf::from("relative/authority")).is_err());
    assert!(CanonicalDirectoryPath::new(root.join("missing-authority")).is_err());
    assert!(opened
        .store()
        .apply_durable(
            0,
            SettingsPatch::Display(DisplayPatch {
                locale: Some("not_a_locale!".to_owned()),
                ..DisplayPatch::default()
            }),
        )
        .is_err());
    assert!(opened
        .store()
        .apply_durable(
            0,
            SettingsPatch::Display(DisplayPatch {
                timezone: Some("Mars/Olympus".to_owned()),
                ..DisplayPatch::default()
            }),
        )
        .is_err());
    let reopened =
        SettingsStore::open(&root).unwrap_or_else(|error| panic!("reopen failed: {error}"));
    assert_eq!(reopened.document(), &SettingsDocument::default());

    drop(reopened);
    drop(opened);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn queued_presentation_patch_can_be_cancelled_without_a_write() {
    let root = isolated_settings_root("settings-cancel");
    let opened = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    opened
        .store()
        .initialize()
        .unwrap_or_else(|error| panic!("initial commit failed: {error}"));
    opened
        .store()
        .queue_presentation(
            0,
            PresentationPatch {
                last_route: ValuePatch::Set("/weekly-review".to_owned()),
                ..PresentationPatch::default()
            },
        )
        .unwrap_or_else(|error| panic!("queue failed: {error}"));

    assert_eq!(
        opened
            .store()
            .cancel_pending_presentation(0)
            .unwrap_or_else(|error| panic!("cancel failed: {error}")),
        CancelOutcome::Cancelled { base_revision: 0 }
    );
    assert_eq!(
        opened
            .store()
            .flush_pending_presentation()
            .unwrap_or_else(|error| panic!("flush failed: {error}")),
        FlushOutcome::NoPending
    );
    let reopened =
        SettingsStore::open(&root).unwrap_or_else(|error| panic!("reopen failed: {error}"));
    assert_eq!(reopened.document(), &SettingsDocument::default());

    drop(reopened);
    drop(opened);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn abrupt_helper_process_exit_cannot_replace_the_canonical_document() {
    let root = isolated_settings_root("settings-abrupt-interruption");
    let opened = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    opened
        .store()
        .initialize()
        .unwrap_or_else(|error| panic!("initial commit failed: {error}"));
    // PMC-REVIEWED-CRASH-HARNESS-START
    let status = Command::new(
        std::env::current_exe().unwrap_or_else(|error| panic!("test executable failed: {error}")),
    )
    .args([
        "--exact",
        "atomic_interruption_process_helper",
        "--nocapture",
    ])
    .env(INTERRUPTION_HELPER_ROOT, root.path())
    .status()
    .unwrap_or_else(|error| panic!("helper launch failed: {error}"));
    // PMC-REVIEWED-CRASH-HARNESS-END
    assert!(!status.success());

    let reopened =
        SettingsStore::open(&root).unwrap_or_else(|error| panic!("reopen failed: {error}"));
    assert_eq!(reopened.disposition(), LoadDisposition::Loaded);
    assert_eq!(reopened.document(), &SettingsDocument::default());

    drop(reopened);
    drop(opened);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn atomic_interruption_process_helper() {
    let Ok(root) = std::env::var(INTERRUPTION_HELPER_ROOT) else {
        return;
    };
    let mut interrupted = AtomicWriteFile::open(PathBuf::from(root).join("settings-v1.json"))
        .unwrap_or_else(|error| panic!("atomic open failed: {error}"));
    interrupted
        .write_all(br#"{"format_version":1"#)
        .unwrap_or_else(|error| panic!("partial write failed: {error}"));
    interrupted
        .sync_all()
        .unwrap_or_else(|error| panic!("temporary flush failed: {error}"));
    // PMC-REVIEWED-ABORT-HARNESS-START
    std::process::abort();
    // PMC-REVIEWED-ABORT-HARNESS-END
}

#[test]
fn semantically_invalid_document_is_preserved_before_safe_fallback() {
    let root = isolated_settings_root("settings-invalid-content");
    fs::create_dir_all(&root).unwrap_or_else(|error| panic!("setup failed: {error}"));
    let invalid = SettingsDocument {
        operational: OperationalSettings {
            retention_days: 0,
            ..OperationalSettings::default()
        },
        ..SettingsDocument::default()
    };
    fs::write(
        root.join("settings-v1.json"),
        serde_json::to_vec(&invalid)
            .unwrap_or_else(|error| panic!("fixture serialization failed: {error}")),
    )
    .unwrap_or_else(|error| panic!("fixture write failed: {error}"));

    let opened = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    assert!(matches!(
        opened.disposition(),
        LoadDisposition::InvalidPreserved {
            reason: InvalidSettingsReason::Schema,
            ..
        }
    ));
    assert_eq!(opened.document(), &SettingsDocument::default());

    drop(opened);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn presentation_queue_after_durable_commit_uses_the_new_revision() {
    let root = isolated_settings_root("settings-durable-before-queue");
    let opened = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    opened
        .store()
        .initialize()
        .unwrap_or_else(|error| panic!("initial commit failed: {error}"));
    assert_eq!(
        opened
            .store()
            .apply_durable(
                0,
                SettingsPatch::Operational(OperationalPatch {
                    retention_days: Some(75),
                    ..OperationalPatch::default()
                }),
            )
            .unwrap_or_else(|error| panic!("durable commit failed: {error}")),
        PatchOutcome::Committed { revision: 1 }
    );
    assert_eq!(
        opened
            .store()
            .queue_presentation(
                1,
                PresentationPatch {
                    sidebar_width: ValuePatch::Set(300),
                    ..PresentationPatch::default()
                },
            )
            .unwrap_or_else(|error| panic!("queue failed: {error}")),
        QueueOutcome::Queued { base_revision: 1 }
    );
    assert_eq!(
        opened
            .store()
            .flush_pending_presentation()
            .unwrap_or_else(|error| panic!("flush failed: {error}")),
        FlushOutcome::Committed { revision: 2 }
    );

    let reopened =
        SettingsStore::open(&root).unwrap_or_else(|error| panic!("reopen failed: {error}"));
    assert_eq!(reopened.document().operational.retention_days, 75);
    assert_eq!(reopened.document().presentation.sidebar_width, Some(300));

    drop(reopened);
    drop(opened);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn cancellation_requires_the_pending_patch_base_revision() {
    let root = isolated_settings_root("settings-cancel-revision");
    let opened = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    opened
        .store()
        .initialize()
        .unwrap_or_else(|error| panic!("initial commit failed: {error}"));
    opened
        .store()
        .queue_presentation(
            0,
            PresentationPatch {
                sidebar_width: ValuePatch::Set(312),
                ..PresentationPatch::default()
            },
        )
        .unwrap_or_else(|error| panic!("queue failed: {error}"));

    assert_eq!(
        opened
            .store()
            .cancel_pending_presentation(9)
            .unwrap_or_else(|error| panic!("cancel failed: {error}")),
        CancelOutcome::RevisionMismatch {
            pending_base_revision: 0
        }
    );
    assert_eq!(
        opened
            .store()
            .flush_pending_presentation()
            .unwrap_or_else(|error| panic!("flush failed: {error}")),
        FlushOutcome::Committed { revision: 1 }
    );

    drop(opened);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn protected_root_rejects_path_traversal_components() {
    assert!(ProtectedSettingsRoot::prepare("..").is_err());
    assert!(ProtectedSettingsRoot::prepare(".").is_err());
    assert!(ProtectedSettingsRoot::prepare("outside/path").is_err());
    assert!(ProtectedSettingsRoot::prepare("C:\\outside").is_err());
}

#[cfg(unix)]
#[test]
fn protected_root_rejects_in_base_and_outside_symlinks_without_mutating_targets() {
    use std::os::unix::fs::{symlink, PermissionsExt};

    let in_base_target = isolated_settings_root("settings-link-target");
    fs::set_permissions(in_base_target.path(), fs::Permissions::from_mode(0o750))
        .unwrap_or_else(|error| panic!("target permission setup failed: {error}"));
    let in_base_name = unique_application_directory("settings-in-base-link");
    let in_base_link = in_base_target
        .path()
        .parent()
        .unwrap_or_else(|| panic!("missing app-data parent"))
        .join(&in_base_name);
    symlink(in_base_target.path(), &in_base_link)
        .unwrap_or_else(|error| panic!("in-base symlink failed: {error}"));
    assert!(CanonicalDirectoryPath::new(in_base_link.clone()).is_err());
    assert!(ProtectedSettingsRoot::prepare(&in_base_name).is_err());
    assert_eq!(
        fs::metadata(in_base_target.path())
            .unwrap_or_else(|error| panic!("target metadata failed: {error}"))
            .permissions()
            .mode()
            & 0o777,
        0o750
    );

    let outside_target = std::env::temp_dir().join(unique_application_directory("outside-target"));
    fs::create_dir(&outside_target)
        .unwrap_or_else(|error| panic!("outside target setup failed: {error}"));
    fs::set_permissions(&outside_target, fs::Permissions::from_mode(0o750))
        .unwrap_or_else(|error| panic!("outside permission setup failed: {error}"));
    let outside_name = unique_application_directory("settings-outside-link");
    let outside_link = in_base_target
        .path()
        .parent()
        .unwrap_or_else(|| panic!("missing app-data parent"))
        .join(&outside_name);
    symlink(&outside_target, &outside_link)
        .unwrap_or_else(|error| panic!("outside symlink failed: {error}"));
    assert!(CanonicalDirectoryPath::new(outside_link.clone()).is_err());
    assert!(ProtectedSettingsRoot::prepare(&outside_name).is_err());
    assert_eq!(
        fs::metadata(&outside_target)
            .unwrap_or_else(|error| panic!("outside metadata failed: {error}"))
            .permissions()
            .mode()
            & 0o777,
        0o750
    );

    fs::remove_file(in_base_link).unwrap_or_else(|error| panic!("link cleanup failed: {error}"));
    fs::remove_file(outside_link).unwrap_or_else(|error| panic!("link cleanup failed: {error}"));
    fs::remove_dir_all(in_base_target)
        .unwrap_or_else(|error| panic!("target cleanup failed: {error}"));
    fs::remove_dir_all(outside_target)
        .unwrap_or_else(|error| panic!("outside cleanup failed: {error}"));
}

#[cfg(unix)]
#[test]
fn invalid_file_preservation_rejects_a_linked_diagnostics_directory() {
    use std::os::unix::fs::symlink;

    let root = isolated_settings_root("settings-linked-diagnostics");
    let target = isolated_settings_root("settings-diagnostic-target");
    let diagnostics = root.join("diagnostics");
    symlink(target.path(), &diagnostics)
        .unwrap_or_else(|error| panic!("diagnostics symlink failed: {error}"));
    let settings_file = root.join("settings-v1.json");
    fs::write(&settings_file, b"{")
        .unwrap_or_else(|error| panic!("invalid fixture failed: {error}"));

    assert!(SettingsStore::open(&root).is_err());
    assert!(settings_file.exists());
    assert_eq!(
        fs::read_dir(target.path())
            .unwrap_or_else(|error| panic!("target read failed: {error}"))
            .count(),
        0
    );

    fs::remove_file(diagnostics)
        .unwrap_or_else(|error| panic!("diagnostics cleanup failed: {error}"));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("root cleanup failed: {error}"));
    fs::remove_dir_all(target).unwrap_or_else(|error| panic!("target cleanup failed: {error}"));
}

#[cfg(target_os = "windows")]
#[test]
fn protected_root_rejects_a_windows_directory_reparse_link() {
    let target = isolated_settings_root("settings-windows-link-target");
    let link_name = unique_application_directory("settings-windows-link");
    let link = target
        .path()
        .parent()
        .unwrap_or_else(|| panic!("missing app-data parent"))
        .join(&link_name);
    // PMC-REVIEWED-JUNCTION-HARNESS-START
    let status = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "New-Item -ItemType Junction -Path $env:PMC_JUNCTION_LINK -Target $env:PMC_JUNCTION_TARGET | Out-Null",
        ])
        .env("PMC_JUNCTION_LINK", &link)
        .env("PMC_JUNCTION_TARGET", target.path())
        .status()
        .unwrap_or_else(|error| panic!("junction helper failed: {error}"));
    // PMC-REVIEWED-JUNCTION-HARNESS-END
    assert!(status.success());
    assert!(CanonicalDirectoryPath::new(link.clone()).is_err());
    assert!(ProtectedSettingsRoot::prepare(&link_name).is_err());

    fs::remove_dir(link).unwrap_or_else(|error| panic!("link cleanup failed: {error}"));
    fs::remove_dir_all(target).unwrap_or_else(|error| panic!("target cleanup failed: {error}"));
}

#[cfg(unix)]
#[test]
fn protected_root_uses_owner_only_unix_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let root = isolated_settings_root("settings-permissions");
    let mode = fs::metadata(root.path())
        .unwrap_or_else(|error| panic!("metadata failed: {error}"))
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn a_stored_folder_that_is_gone_does_not_make_the_settings_unreadable() {
    // A backup folder on a removable drive that is not plugged in: the
    // settings still load, and only use-time revalidation reports it.
    let root = isolated_settings_root("settings-folder-gone");
    let opened = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    opened
        .store()
        .initialize_with_locale("ja")
        .unwrap_or_else(|error| panic!("initialize failed: {error}"));
    let backup = root.join("removable-backup");
    fs::create_dir(&backup).unwrap_or_else(|error| panic!("fixture failed: {error}"));
    let destination = CanonicalDirectoryPath::new(backup.clone())
        .unwrap_or_else(|error| panic!("destination failed: {error}"));
    opened
        .store()
        .apply_durable(
            0,
            SettingsPatch::Operational(OperationalPatch {
                backup_destination: ValuePatch::Set(destination.into()),
                ..OperationalPatch::default()
            }),
        )
        .unwrap_or_else(|error| panic!("patch failed: {error}"));
    drop(opened);
    fs::remove_dir(&backup).unwrap_or_else(|error| panic!("unplug failed: {error}"));

    let reopened =
        SettingsStore::open(&root).unwrap_or_else(|error| panic!("reopen failed: {error}"));
    assert_eq!(reopened.disposition(), LoadDisposition::Loaded);
    assert_eq!(reopened.document().display.locale, "ja");
    let stored = reopened
        .document()
        .operational
        .backup_destination
        .clone()
        .unwrap_or_else(|| panic!("the destination must still be stored"));
    assert!(stored.revalidate().is_err());

    fs::create_dir(&backup).unwrap_or_else(|error| panic!("replug failed: {error}"));
    assert!(stored.revalidate().is_ok());

    drop(reopened);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn a_stored_folder_must_still_be_absolute_and_normalized_to_load() {
    let root = isolated_settings_root("settings-folder-syntax");
    fs::create_dir_all(&root).unwrap_or_else(|error| panic!("setup failed: {error}"));
    let body = r#"{"format_version":1,"revision":0,"display":{"locale":"en","timezone":"Asia/Taipei","theme":"system"},"operational":{"authority_root":null,"backup_destination":"relative\\backup","live_vault_root":null,"retention_days":30,"external_ai_enabled":false,"log_level":"info"},"presentation":{"last_route":null,"sidebar_width":null,"window":null,"table_preferences":[]}}"#;
    fs::write(root.join("settings-v1.json"), body)
        .unwrap_or_else(|error| panic!("write failed: {error}"));
    let reopened =
        SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    assert!(matches!(
        reopened.disposition(),
        LoadDisposition::InvalidPreserved { .. }
    ));
    drop(reopened);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn a_stored_authority_root_is_still_proven_when_settings_load() {
    // Only the backup destination may be absent at load; the authority root
    // keeps its filesystem proof, so a missing one is refused as before.
    let root = isolated_settings_root("settings-authority-strict");
    fs::create_dir_all(&root).unwrap_or_else(|error| panic!("setup failed: {error}"));
    let missing = root.join("no-such-authority");
    let body = format!(
        r#"{{"format_version":1,"revision":0,"display":{{"locale":"en","timezone":"Asia/Taipei","theme":"system"}},"operational":{{"authority_root":{},"backup_destination":null,"live_vault_root":null,"retention_days":30,"external_ai_enabled":false,"log_level":"info"}},"presentation":{{"last_route":null,"sidebar_width":null,"window":null,"table_preferences":[]}}}}"#,
        serde_json::to_string(&missing).unwrap_or_default()
    );
    fs::write(root.join("settings-v1.json"), body)
        .unwrap_or_else(|error| panic!("write failed: {error}"));
    let reopened =
        SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    assert!(matches!(
        reopened.disposition(),
        LoadDisposition::InvalidPreserved { .. }
    ));
    drop(reopened);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn the_backup_export_holds_only_portable_non_secret_settings() {
    let root = isolated_settings_root("settings-backup-export");
    let opened = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    opened
        .store()
        .initialize_with_locale("ko")
        .unwrap_or_else(|error| panic!("initialize failed: {error}"));
    fs::create_dir(root.join("backup")).unwrap_or_else(|error| panic!("fixture failed: {error}"));
    let destination = CanonicalDirectoryPath::new(root.join("backup"))
        .unwrap_or_else(|error| panic!("destination failed: {error}"));
    opened
        .store()
        .apply_durable(
            0,
            SettingsPatch::Operational(OperationalPatch {
                backup_destination: ValuePatch::Set(destination.into()),
                ..OperationalPatch::default()
            }),
        )
        .unwrap_or_else(|error| panic!("patch failed: {error}"));
    let document = opened
        .store()
        .read()
        .unwrap_or_else(|error| panic!("read failed: {error}"));
    let export = String::from_utf8(
        document
            .backup_export()
            .unwrap_or_else(|error| panic!("export failed: {error}")),
    )
    .unwrap_or_default();
    assert!(
        export.contains("\"format\":\"pmc-backup-settings/v1\""),
        "{export}"
    );
    assert!(export.contains("\"locale\":\"ko\""), "{export}");
    assert!(
        !export.contains("destination"),
        "no destination field: {export}"
    );
    assert!(!export.contains("authority"), "no authority path: {export}");
    assert!(!export.contains(&root.display().to_string().replace('\\', "\\\\")));
    drop(opened);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn a_restore_applies_exactly_the_six_archived_settings_in_one_write() {
    let root = isolated_settings_root("settings-restore");
    let store = SettingsStore::open(&root)
        .unwrap_or_else(|error| panic!("open failed: {error}"))
        .into_store();
    store
        .initialize()
        .unwrap_or_else(|error| panic!("initialize failed: {error}"));
    let before = store.read().unwrap_or_else(|error| panic!("{error}"));
    let archived = BackupSettings::parse(
        br#"{"format":"pmc-backup-settings/v1","locale":"ja","timezone":"Asia/Tokyo","theme":"dark","retention_days":60,"external_ai_enabled":true,"log_level":"warn"}"#,
    )
    .unwrap_or_else(|error| panic!("parse failed: {error}"));
    let outcome = store
        .apply_restore_export(before.revision, &archived)
        .unwrap_or_else(|error| panic!("apply failed: {error}"));
    assert_eq!(
        outcome,
        PatchOutcome::Committed {
            revision: before.revision + 1
        }
    );
    let after = store.read().unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(after.display.locale, "ja");
    assert_eq!(after.display.timezone, "Asia/Tokyo");
    assert_eq!(after.operational.retention_days, 60);
    assert!(after.operational.external_ai_enabled);
    // Machine-bound settings are not in a backup and are not touched.
    assert_eq!(
        after.operational.authority_root,
        before.operational.authority_root
    );
    assert_eq!(
        after.operational.backup_destination,
        before.operational.backup_destination
    );
    assert_eq!(after.presentation, before.presentation);

    // And the rollback puts every field back, with the revision moving on.
    store
        .put_back(after.revision, &before)
        .unwrap_or_else(|error| panic!("put back failed: {error}"));
    let restored = store.read().unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(restored.display, before.display);
    assert_eq!(restored.operational, before.operational);
    assert_eq!(restored.revision, after.revision + 1);
}

#[test]
fn an_archived_settings_file_with_anything_unexpected_is_refused() {
    for bytes in [
        br#"{"format":"pmc-backup-settings/v2","locale":"ja","timezone":"Asia/Tokyo","theme":"dark","retention_days":60,"external_ai_enabled":true,"log_level":"warn"}"#.as_slice(),
        br#"{"format":"pmc-backup-settings/v1","locale":"ja","timezone":"Asia/Tokyo","theme":"dark","retention_days":60,"external_ai_enabled":true,"log_level":"warn","backup_destination":"C:\\x"}"#.as_slice(),
        br#"{"format":"pmc-backup-settings/v1","locale":"","timezone":"Asia/Tokyo","theme":"dark","retention_days":60,"external_ai_enabled":true,"log_level":"warn"}"#.as_slice(),
    ] {
        assert!(BackupSettings::parse(bytes).is_err());
    }
}

#[test]
fn a_restore_write_on_a_stale_revision_changes_nothing() {
    let root = isolated_settings_root("settings-restore-stale");
    let store = SettingsStore::open(&root)
        .unwrap_or_else(|error| panic!("open failed: {error}"))
        .into_store();
    store
        .initialize()
        .unwrap_or_else(|error| panic!("initialize failed: {error}"));
    let before = store.read().unwrap_or_else(|error| panic!("{error}"));
    let archived = BackupSettings::parse(
        br#"{"format":"pmc-backup-settings/v1","locale":"ko","timezone":"Asia/Seoul","theme":"light","retention_days":30,"external_ai_enabled":false,"log_level":"info"}"#,
    )
    .unwrap_or_else(|error| panic!("parse failed: {error}"));
    assert!(matches!(
        store.apply_restore_export(before.revision + 5, &archived),
        Ok(PatchOutcome::Stale { .. })
    ));
    assert_eq!(
        store.read().unwrap_or_else(|error| panic!("{error}")),
        before
    );
}

#[test]
fn peek_locale_reads_the_document_as_it_is_and_initialises_nothing() {
    let root = isolated_settings_root("peek-locale");
    // No document yet: nothing is learned and nothing is created.
    assert_eq!(pmc_platform::settings::peek_locale(&root), None);
    assert!(
        std::fs::read_dir(root.path())
            .map(|entries| entries.count())
            .unwrap_or(usize::MAX)
            == 0,
        "a peek must not create the settings document"
    );
    let opened = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    opened
        .store()
        .initialize_with_locale("ja")
        .unwrap_or_else(|error| panic!("initialize failed: {error}"));
    assert_eq!(
        pmc_platform::settings::peek_locale(&root).as_deref(),
        Some("ja")
    );
}

#[test]
fn a_format_1_document_is_upgraded_in_place_and_keeps_every_setting_and_its_revision() {
    // Item ⑦ added `operational.live_vault_root`, so the document format
    // moved 1 -> 2. A document written by an earlier PMC must open, not be
    // set aside: it gains the new field as absent (no Vault configured) and
    // is written back at the current version, with nothing the person set
    // changed and the revision where it was.
    let root = isolated_settings_root("settings-format-1-upgrade");
    fs::create_dir_all(&root).unwrap_or_else(|error| panic!("setup failed: {error}"));
    let path = root.join("settings-v1.json");
    fs::write(
        &path,
        br#"{"format_version":1,"revision":7,"display":{"locale":"ja","timezone":"Asia/Tokyo","theme":"dark"},"operational":{"authority_root":null,"backup_destination":null,"retention_days":45,"external_ai_enabled":true,"log_level":"debug"},"presentation":{"last_route":"/work-queue","sidebar_width":null,"window":null,"table_preferences":[]}}"#,
    )
    .unwrap_or_else(|error| panic!("fixture write failed: {error}"));

    let opened = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    // Not merely `Loaded`: the caller is told the document was upgraded.
    assert_eq!(opened.disposition(), LoadDisposition::Upgraded);
    let document = opened.document();
    assert_eq!(document.format_version, 3);
    assert_eq!(document.revision, 7);
    assert_eq!(document.display.locale, "ja");
    assert_eq!(document.display.timezone, "Asia/Tokyo");
    assert_eq!(document.operational.retention_days, 45);
    assert!(document.operational.external_ai_enabled);
    assert_eq!(document.operational.live_vault_root, None);
    // Written by a PMC that only ever opened Live (item ⑨).
    assert_eq!(
        document.operational.selected_workspace,
        Some(SelectedWorkspace::Live)
    );
    assert_eq!(
        document.presentation.last_route.as_deref(),
        Some("/work-queue")
    );

    // Written back, so the next open reads the current format directly.
    let on_disk = fs::read(&path).unwrap_or_else(|error| panic!("read back failed: {error}"));
    let stored: serde_json::Value =
        serde_json::from_slice(&on_disk).unwrap_or_else(|error| panic!("reparse failed: {error}"));
    assert_eq!(stored["format_version"], 3);
    assert_eq!(stored["revision"], 7);
    assert!(stored["operational"]["live_vault_root"].is_null());
    assert_eq!(stored["operational"]["selected_workspace"], "live");

    drop(opened);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn a_live_vault_root_survives_a_close_and_reopen_and_needs_no_filesystem_to_load() {
    let root = isolated_settings_root("settings-live-vault-root");
    let opened = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    opened
        .store()
        .initialize()
        .unwrap_or_else(|error| panic!("initialize failed: {error}"));
    // A folder that does not exist: a Vault on a drive that is not plugged
    // in must still load, and be reported unavailable rather than unset.
    let absent = root.path().join("vault-on-another-drive");
    let folder = DeferredDirectoryPath::from(
        CanonicalDirectoryPath::new({
            fs::create_dir_all(&absent).unwrap_or_else(|error| panic!("setup failed: {error}"));
            fs::canonicalize(&absent).unwrap_or_else(|error| panic!("canonicalize failed: {error}"))
        })
        .unwrap_or_else(|error| panic!("path failed: {error}")),
    );
    let revision = opened
        .store()
        .read()
        .unwrap_or_else(|error| panic!("read failed: {error}"))
        .revision;
    opened
        .store()
        .apply_durable(
            revision,
            SettingsPatch::Operational(OperationalPatch {
                live_vault_root: ValuePatch::Set(folder.clone()),
                ..OperationalPatch::default()
            }),
        )
        .unwrap_or_else(|error| panic!("commit failed: {error}"));
    drop(opened);
    fs::remove_dir_all(&absent).unwrap_or_else(|error| panic!("remove failed: {error}"));

    let reopened =
        SettingsStore::open(&root).unwrap_or_else(|error| panic!("reopen failed: {error}"));
    assert_eq!(reopened.disposition(), LoadDisposition::Loaded);
    let stored = reopened
        .document()
        .operational
        .live_vault_root
        .clone()
        .unwrap_or_else(|| panic!("the Vault folder was not kept"));
    assert_eq!(stored, folder);
    assert!(
        stored.revalidate().is_err(),
        "a folder that is gone must not revalidate"
    );

    drop(reopened);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn a_format_1_preimage_is_still_readable_after_the_binary_moved_to_format_2() {
    // A restore prepared by an earlier PMC keeps the settings it will put
    // back as JSON. Refusing to read that after an upgrade would turn a
    // recoverable restore into an unrecoverable one.
    let body = br#"{"format_version":1,"revision":4,"display":{"locale":"en","timezone":"Asia/Taipei","theme":"system"},"operational":{"authority_root":null,"backup_destination":null,"retention_days":30,"external_ai_enabled":false,"log_level":"info"},"presentation":{"last_route":null,"sidebar_width":null,"window":null,"table_preferences":[]}}"#;
    let document = pmc_platform::settings::document_from_json(body)
        .unwrap_or_else(|error| panic!("preimage read failed: {error}"));
    assert_eq!(document.format_version, 3);
    assert_eq!(document.revision, 4);
    assert_eq!(document.display.locale, "en");
    assert_eq!(document.operational.live_vault_root, None);
    assert_eq!(
        document.operational.selected_workspace,
        Some(SelectedWorkspace::Live)
    );
}

#[test]
fn a_drive_root_is_named_by_its_drive_rather_than_by_nothing() {
    // §2 shows the chosen folder's own name; a drive root has no last
    // component, and "nothing" is not a name a person can recognise.
    let root = isolated_settings_root("drive-root-name");
    let drive = root
        .path()
        .components()
        .next()
        .unwrap_or_else(|| panic!("no prefix component"))
        .as_os_str()
        .to_string_lossy()
        .into_owned();
    let stored: DeferredDirectoryPath = serde_json::from_str(
        &serde_json::to_string(&format!("{drive}\\"))
            .unwrap_or_else(|error| panic!("json: {error}")),
    )
    .unwrap_or_else(|error| panic!("stored path: {error}"));
    assert_eq!(
        stored.folder_name().as_deref(),
        Some(drive.trim_end_matches('\\'))
    );
    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn folders_overlap_by_components_not_by_string_prefix() {
    // A Vault must stay away from the folders PMC owns (item ⑦, §2): inside,
    // around or equal all count; a sibling that merely shares a prefix does
    // not.
    let root = isolated_settings_root("overlap");
    let base = root.path().to_path_buf();
    for name in ["Vault", "Vault2", "Vault/inner"] {
        fs::create_dir_all(base.join(name)).unwrap_or_else(|error| panic!("setup: {error}"));
    }
    let canonical = |name: &str| {
        CanonicalDirectoryPath::new(
            fs::canonicalize(base.join(name)).unwrap_or_else(|error| panic!("canon: {error}")),
        )
        .unwrap_or_else(|error| panic!("path: {error}"))
    };
    let vault = canonical("Vault");
    let inner = canonical("Vault/inner");
    let sibling = canonical("Vault2");
    assert!(vault.overlaps(inner.as_path()));
    assert!(inner.overlaps(vault.as_path()));
    assert!(vault.overlaps(vault.as_path()));
    assert!(!sibling.overlaps(vault.as_path()));
    assert!(!vault.overlaps(sibling.as_path()));
    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn a_format_2_document_upgrades_to_format_3_as_live_and_keeps_its_vault_folder() {
    // Item ⑨: format 2 shipped with the Vault folder alone. Every profile
    // that has one has used PMC, so it opens Live and is never asked.
    let root = isolated_settings_root("settings-format-2-upgrade");
    fs::create_dir_all(&root).unwrap_or_else(|error| panic!("setup failed: {error}"));
    let vault = root.join("vault");
    fs::create_dir_all(&vault).unwrap_or_else(|error| panic!("setup failed: {error}"));
    let vault = fs::canonicalize(&vault).unwrap_or_else(|error| panic!("canonicalize: {error}"));
    let body = format!(
        r#"{{"format_version":2,"revision":9,"display":{{"locale":"en","timezone":"Asia/Taipei","theme":"system"}},"operational":{{"authority_root":null,"backup_destination":null,"live_vault_root":{},"retention_days":30,"external_ai_enabled":false,"log_level":"info"}},"presentation":{{"last_route":null,"sidebar_width":null,"window":null,"table_preferences":[]}}}}"#,
        serde_json::to_string(&vault).unwrap_or_else(|error| panic!("json: {error}"))
    );
    fs::write(root.join("settings-v1.json"), body)
        .unwrap_or_else(|error| panic!("fixture write failed: {error}"));

    let opened = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    assert_eq!(opened.disposition(), LoadDisposition::Upgraded);
    let document = opened.document();
    assert_eq!(document.format_version, 3);
    assert_eq!(document.revision, 9);
    assert_eq!(
        document.operational.selected_workspace,
        Some(SelectedWorkspace::Live)
    );
    assert!(document.operational.live_vault_root.is_some());
    drop(opened);

    let reopened =
        SettingsStore::open(&root).unwrap_or_else(|error| panic!("reopen failed: {error}"));
    assert_eq!(reopened.disposition(), LoadDisposition::Loaded);
    drop(reopened);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn a_new_profile_stays_unchosen_until_a_choice_is_recorded_and_never_goes_back() {
    let root = isolated_settings_root("settings-first-run-choice");
    let opened = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    opened
        .store()
        .initialize_first_run(FOLLOW_SYSTEM_LOCALE, None)
        .unwrap_or_else(|error| panic!("initialize failed: {error}"));
    drop(opened);

    let reopened =
        SettingsStore::open(&root).unwrap_or_else(|error| panic!("reopen failed: {error}"));
    assert_eq!(reopened.disposition(), LoadDisposition::Loaded);
    assert_eq!(reopened.document().operational.selected_workspace, None);
    let store = reopened.into_store();
    let revision = store
        .read()
        .unwrap_or_else(|error| panic!("read failed: {error}"))
        .revision;
    // Stale against another writer: nothing recorded.
    assert!(matches!(
        store.select_workspace(revision + 1, SelectedWorkspace::Training),
        Ok(PatchOutcome::Stale { .. })
    ));
    assert!(matches!(
        store.select_workspace(revision, SelectedWorkspace::Training),
        Ok(PatchOutcome::Committed { .. })
    ));
    let document = store
        .read()
        .unwrap_or_else(|error| panic!("read failed: {error}"));
    assert_eq!(
        document.operational.selected_workspace,
        Some(SelectedWorkspace::Training)
    );
    assert_eq!(document.revision, revision + 1);
    drop(store);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn neither_a_backup_nor_a_restore_rollback_carries_the_selected_workspace() {
    let root = isolated_settings_root("settings-selection-not-backed-up");
    let opened = SettingsStore::open(&root).unwrap_or_else(|error| panic!("open failed: {error}"));
    opened
        .store()
        .initialize_first_run("en", Some(SelectedWorkspace::Live))
        .unwrap_or_else(|error| panic!("initialize failed: {error}"));
    let store = opened.into_store();
    let preimage = store
        .read()
        .unwrap_or_else(|error| panic!("read failed: {error}"));
    let export: serde_json::Value = serde_json::from_slice(
        &preimage
            .backup_export()
            .unwrap_or_else(|error| panic!("export failed: {error}")),
    )
    .unwrap_or_else(|error| panic!("reparse failed: {error}"));
    assert!(export.get("selected_workspace").is_none());

    assert!(matches!(
        store.select_workspace(preimage.revision, SelectedWorkspace::Training),
        Ok(PatchOutcome::Committed { .. })
    ));
    // A rollback puts back every other field, but not which workspace opens.
    assert!(matches!(
        store.put_back(preimage.revision + 1, &preimage),
        Ok(PatchOutcome::Committed { .. })
    ));
    assert_eq!(
        store
            .read()
            .unwrap_or_else(|error| panic!("read failed: {error}"))
            .operational
            .selected_workspace,
        Some(SelectedWorkspace::Training)
    );
    drop(store);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("cleanup failed: {error}"));
}

#[test]
fn only_a_never_used_profile_is_asked_and_unreadable_settings_open_live() {
    use pmc_platform::settings::InvalidSettingsReason::Schema;
    use StartupSelection::{Choose, Open};
    let live = SelectedWorkspace::Live;
    let training = SelectedWorkspace::Training;
    let invalid = LoadDisposition::InvalidPreserved {
        path: PathBuf::from("settings.invalid.json"),
        reason: Schema,
    };
    // Unreadable settings: Live, whatever else is true.
    assert_eq!(startup_selection(&invalid, None, false), Open(live));
    assert_eq!(
        startup_selection(&invalid, Some(training), false),
        Open(live)
    );
    for disposition in [
        LoadDisposition::Loaded,
        LoadDisposition::Upgraded,
        LoadDisposition::MissingDefaulted,
    ] {
        assert_eq!(
            startup_selection(&disposition, Some(training), false),
            Open(training)
        );
        assert_eq!(
            startup_selection(&disposition, Some(live), false),
            Open(live)
        );
        // Not chosen, but Live exists: this person has used PMC.
        assert_eq!(startup_selection(&disposition, None, true), Open(live));
        assert_eq!(startup_selection(&disposition, None, false), Choose);
    }
}
