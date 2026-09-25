//! The restore control record: empty until written, replaced whole under a
//! lock, and a refused change writes nothing.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_platform::restore_control::{
    CurrentSourceBinding, FileMemberBinding, FileSetBinding, LedgerBinding, LedgerFileMember,
    PreparedRestore, RecoveryEvidence, RestoreControl, RestoreControlError, RestoreControlStore,
    RestoreOperation, RestoreOutcome, RestorePhase, RestoreTerminal, UnopenedReason,
};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static LAST_PATH: std::cell::RefCell<std::path::PathBuf> =
        std::cell::RefCell::new(std::path::PathBuf::new());
}

fn store_path(_store: &RestoreControlStore) -> std::path::PathBuf {
    LAST_PATH.with(|path| path.borrow().clone())
}

fn store() -> RestoreControlStore {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir()
        .join(format!("pmc-restore-control-{nonce}-{sequence}"))
        .join("restore-control-live-v1.json");
    LAST_PATH.with(|last| *last.borrow_mut() = path.clone());
    RestoreControlStore::new(path)
}

fn prepared() -> PreparedRestore {
    PreparedRestore {
        prepared_intent_id: "restore-intent-1".to_owned(),
        payload_sha256: "a".repeat(64),
        archive_id: "a1a1a1a1".to_owned(),
        archive_container_sha256: "b".repeat(64),
        archive_created_at: "2026-09-22T03:59:34.792Z".to_owned(),
        archive_schema_version: 46,
        archive_ledger_revision: 156,
        archive_record_count: 139,
        snapshot_sha256: "c".repeat(64),
        settings_member_sha256: "f".repeat(64),
        inventory_member_sha256: "9".repeat(64),
        current: CurrentSourceBinding::Inspectable {
            binding: LedgerBinding {
                schema_version: 46,
                ledger_revision: 0,
                inventory_sha256: "d".repeat(64),
                content_sha256: "e".repeat(64),
            },
        },
        unopened_reason: None,
        settings_revision: 3,
        recovery: RecoveryEvidence::OperationalBackup {
            archive_id: "b2b2b2b2".to_owned(),
        },
        confirmation_date: "2026-09-22".to_owned(),
        confirmation_timezone: "Asia/Taipei".to_owned(),
        prepared_at_millis: 1_000,
        expires_at_millis: 2_000,
    }
}

#[test]
fn the_record_starts_empty_and_round_trips_every_state() {
    let store = store();
    assert_eq!(
        store.load().unwrap_or_else(|error| panic!("{error}")),
        RestoreControl::default()
    );
    store
        .update(|control| -> Result<(), ()> {
            control.active = Some(RestoreOperation::Executing {
                prepared: prepared(),
                idempotency_id: "restore-execute-1".to_owned(),
                receipt_id: "receipt-1".to_owned(),
                approved_at_millis: 1_500,
                phase: RestorePhase::Installing,
            });
            control.finished.push(RestoreTerminal {
                idempotency_id: "restore-execute-0".to_owned(),
                prepared_intent_id: "restore-intent-0".to_owned(),
                outcome: RestoreOutcome::FailedBeforeReplacement,
                recovery: RecoveryEvidence::OperationalBackup {
                    archive_id: "b0b0b0b0".to_owned(),
                },
                completed_at_millis: 900,
                projections_pending_for: None,
                resolved_recovery_failure: None,
            });
            Ok(())
        })
        .unwrap_or_else(|error| panic!("{error}"))
        .unwrap_or_else(|()| panic!("refused"));
    let loaded = store.load().unwrap_or_else(|error| panic!("{error}"));
    assert!(matches!(
        loaded.active,
        Some(RestoreOperation::Executing {
            phase: RestorePhase::Installing,
            ..
        })
    ));
    assert_eq!(loaded.finished.len(), 1);
}

#[test]
fn a_refused_change_writes_nothing() {
    let store = store();
    store
        .update(|control| -> Result<(), ()> {
            control.active = Some(RestoreOperation::Prepared {
                prepared: prepared(),
            });
            Ok(())
        })
        .unwrap_or_else(|error| panic!("{error}"))
        .unwrap_or_else(|()| panic!("refused"));
    let before = store.load().unwrap_or_else(|error| panic!("{error}"));
    let refused = store
        .update(|control| {
            control.active = None;
            Err::<(), &str>("the digest does not match")
        })
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(refused, Err("the digest does not match"));
    assert_eq!(
        store.load().unwrap_or_else(|error| panic!("{error}")),
        before
    );
}

#[test]
fn only_one_of_two_concurrent_claims_succeeds() {
    let store = store();
    store
        .update(|control| -> Result<(), ()> {
            control.active = Some(RestoreOperation::Prepared {
                prepared: prepared(),
            });
            Ok(())
        })
        .unwrap_or_else(|error| panic!("{error}"))
        .unwrap_or_else(|()| panic!("refused"));
    let claim = |id: &'static str| {
        let store = store.clone();
        std::thread::spawn(move || {
            store
                .update(|control| match control.active.take() {
                    Some(RestoreOperation::Prepared { prepared }) => {
                        control.active = Some(RestoreOperation::Executing {
                            prepared,
                            idempotency_id: id.to_owned(),
                            receipt_id: format!("receipt-{id}"),
                            approved_at_millis: 1_500,
                            phase: RestorePhase::Approved,
                        });
                        Ok(())
                    }
                    other => {
                        control.active = other;
                        Err(())
                    }
                })
                .unwrap_or_else(|error| panic!("{error}"))
                .is_ok()
        })
    };
    let first = claim("execute-a");
    let second = claim("execute-b");
    let claimed = [first, second]
        .into_iter()
        .map(|handle| handle.join().unwrap_or_else(|_| panic!("panicked")))
        .filter(|claimed| *claimed)
        .count();
    assert_eq!(claimed, 1);
}

fn member(member: LedgerFileMember, length: u64, fill: char) -> FileMemberBinding {
    FileMemberBinding {
        member,
        length,
        sha256: fill.to_string().repeat(64),
    }
}

/// A v1 document as 8e wrote it: an executing restore over an open Ledger
/// and one finished one.
const V1: &str = r#"{
  "format": "pmc-restore-control/v1",
  "active": {
    "state": "executing",
    "prepared": {
      "prepared_intent_id": "restore-intent-1",
      "payload_sha256": "aaaa",
      "archive_id": "a1a1a1a1",
      "archive_container_sha256": "bbbb",
      "archive_created_at": "2026-09-22T03:59:34.792Z",
      "archive_schema_version": 46,
      "archive_ledger_revision": 156,
      "archive_record_count": 139,
      "snapshot_sha256": "cccc",
      "settings_member_sha256": "ffff",
      "inventory_member_sha256": "9999",
      "current": {
        "schema_version": 47,
        "ledger_revision": 12,
        "inventory_sha256": "dddd",
        "content_sha256": "eeee"
      },
      "settings_revision": 3,
      "recovery_archive_id": "b2b2b2b2",
      "confirmation_date": "2026-09-22",
      "confirmation_timezone": "Asia/Taipei",
      "prepared_at_millis": 1000,
      "expires_at_millis": 2000
    },
    "idempotency_id": "restore-execute-1",
    "receipt_id": "receipt-1",
    "approved_at_millis": 1500,
    "phase": "moving_live_aside"
  },
  "finished": [
    {
      "idempotency_id": "restore-execute-0",
      "prepared_intent_id": "restore-intent-0",
      "outcome": "recovery_failed",
      "recovery_archive_id": "b0b0b0b0",
      "completed_at_millis": 900,
      "projections_pending_for": null
    }
  ]
}"#;

fn store_with(bytes: &str) -> RestoreControlStore {
    let path = std::env::temp_dir().join(format!(
        "pmc-restore-control-raw-{}",
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&path).unwrap_or_else(|error| panic!("{error}"));
    let file = path.join("restore-control-live-v1.json");
    std::fs::write(&file, bytes).unwrap_or_else(|error| panic!("{error}"));
    RestoreControlStore::new(file)
}

#[test]
fn a_v1_record_reads_as_the_v2_it_means_and_is_written_as_v2() {
    let store = store_with(V1);
    let control = store.load().unwrap_or_else(|error| panic!("{error}"));
    let Some(RestoreOperation::Executing {
        prepared, phase, ..
    }) = &control.active
    else {
        panic!("the executing restore must survive");
    };
    assert_eq!(*phase, RestorePhase::MovingLiveAside);
    // v1 only ever restored over an open Ledger, kept by a backup.
    assert_eq!(
        prepared.current,
        CurrentSourceBinding::Inspectable {
            binding: LedgerBinding {
                schema_version: 47,
                ledger_revision: 12,
                inventory_sha256: "dddd".to_owned(),
                content_sha256: "eeee".to_owned(),
            },
        }
    );
    assert_eq!(prepared.unopened_reason, None);
    assert_eq!(
        prepared.recovery,
        RecoveryEvidence::OperationalBackup {
            archive_id: "b2b2b2b2".to_owned(),
        }
    );
    assert_eq!(control.finished.len(), 1);
    assert_eq!(control.finished[0].outcome, RestoreOutcome::RecoveryFailed);
    assert_eq!(control.finished[0].recovery.identity(), "b0b0b0b0");
    assert_eq!(control.finished[0].resolved_recovery_failure, None);

    // The next change writes v2, with nothing lost.
    store
        .update(|_| -> Result<(), ()> { Ok(()) })
        .unwrap_or_else(|error| panic!("{error}"))
        .unwrap_or_else(|()| panic!("refused"));
    let reread = store.load().unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(reread.format, "pmc-restore-control/v2");
    assert_eq!(reread.active, control.active);
    assert_eq!(reread.finished, control.finished);
}

#[test]
fn an_unknown_or_broken_record_is_unreadable_and_never_overwritten() {
    for bytes in [
        r#"{"format":"pmc-restore-control/v9","active":null,"finished":[]}"#,
        r#"{"format":"pmc-restore-control/v1","active":null,"finished":[],"extra":1}"#,
        "not json",
    ] {
        let store = store_with(bytes);
        assert!(matches!(store.load(), Err(RestoreControlError::Unreadable)));
        assert!(store.update(|_| -> Result<(), ()> { Ok(()) }).is_err());
    }
}

#[test]
fn a_file_set_binding_is_the_same_whatever_order_and_changes_with_any_member() {
    let ledger = member(LedgerFileMember::Ledger, 4096, 'a');
    let wal = member(LedgerFileMember::Wal, 512, 'b');
    let shm = member(LedgerFileMember::Shm, 32768, 'c');
    let ordered = FileSetBinding::new(vec![ledger.clone(), wal.clone(), shm.clone()])
        .unwrap_or_else(|| panic!("binding"));
    let shuffled = FileSetBinding::new(vec![shm.clone(), ledger.clone(), wal.clone()])
        .unwrap_or_else(|| panic!("binding"));
    assert_eq!(ordered, shuffled);
    assert!(ordered.is_consistent());

    let variants = [
        vec![ledger.clone(), wal.clone()],
        vec![
            ledger.clone(),
            member(LedgerFileMember::Wal, 513, 'b'),
            shm.clone(),
        ],
        vec![
            ledger.clone(),
            member(LedgerFileMember::Wal, 512, 'd'),
            shm.clone(),
        ],
    ];
    for members in variants {
        let other = FileSetBinding::new(members).unwrap_or_else(|| panic!("binding"));
        assert_ne!(other.aggregate_sha256(), ordered.aggregate_sha256());
    }
    // No main Ledger file, or a member twice: not a Ledger's file set.
    assert!(FileSetBinding::new(vec![wal.clone(), shm]).is_none());
    assert!(FileSetBinding::new(vec![ledger, wal.clone(), wal]).is_none());
}

#[test]
fn an_opaque_source_and_a_preservation_copy_round_trip_and_a_forged_digest_is_refused() {
    let files = FileSetBinding::new(vec![
        member(LedgerFileMember::Ledger, 4096, 'a'),
        member(LedgerFileMember::Wal, 512, 'b'),
    ])
    .unwrap_or_else(|| panic!("binding"));
    let mut opaque = prepared();
    opaque.current = CurrentSourceBinding::Opaque {
        files: files.clone(),
    };
    opaque.unopened_reason = Some(UnopenedReason::OpenFailed);
    opaque.recovery = RecoveryEvidence::PreservationCopy {
        preservation_id: "p1p1p1p1".to_owned(),
        file_name: "pmc-preservation-20260923T010203000Z-p1p1p1p1-v1.tar.zst.age".to_owned(),
        files: files.clone(),
    };
    let store = store();
    store
        .update(|control| -> Result<(), ()> {
            control.active = Some(RestoreOperation::Prepared {
                prepared: opaque.clone(),
            });
            Ok(())
        })
        .unwrap_or_else(|error| panic!("{error}"))
        .unwrap_or_else(|()| panic!("refused"));
    assert_eq!(
        store
            .load()
            .unwrap_or_else(|error| panic!("{error}"))
            .active,
        Some(RestoreOperation::Prepared { prepared: opaque })
    );

    // A stored binding whose digest does not match its members: unreadable.
    let written =
        std::fs::read_to_string(store_path(&store)).unwrap_or_else(|error| panic!("{error}"));
    let forged = written.replace(files.aggregate_sha256(), &"0".repeat(64));
    assert_ne!(forged, written);
    let tampered = store_with(&forged);
    assert!(matches!(
        tampered.load(),
        Err(RestoreControlError::Unreadable)
    ));
}

#[test]
fn a_v1_prepared_intent_is_dropped_so_it_can_never_be_approved() {
    // Prepared under the v1 preview text, which this version no longer
    // produces: read as nothing prepared, like a restart.
    let prepared_only = V1
        .replace("\"state\": \"executing\"", "\"state\": \"prepared\"")
        .replace(
            "    \"idempotency_id\": \"restore-execute-1\",\n    \"receipt_id\": \"receipt-1\",\n    \"approved_at_millis\": 1500,\n    \"phase\": \"moving_live_aside\"\n",
            "",
        )
        .replace("    },\n  },\n  \"finished\"", "    }\n  },\n  \"finished\"");
    assert!(prepared_only.contains("\"state\": \"prepared\""));
    assert!(!prepared_only.contains("receipt-1"));
    let control = store_with(&prepared_only)
        .load()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(control.active, None);
    // The finished history is kept.
    assert_eq!(control.finished.len(), 1);
}
