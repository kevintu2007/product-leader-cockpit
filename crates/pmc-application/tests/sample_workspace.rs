//! The sample workspace's preparation, reset and delete (item ⑨; the
//! accepted sample-workspace amendment §7 and §8): reuse a complete current
//! sample, rebuild beside it and swap, refuse anything PMC did not write,
//! never seed Live, and finish or roll back an interrupted operation from
//! what is on disk.
//!
//! Every test uses its own protected application directory under the OS
//! app-data base, so the real desktop directory is never touched.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_application::sample_lifecycle::{
    ApproveSampleDelete, PreparedSample, SampleError, SampleWorkspace,
};
use pmc_application::sample_workspace::{
    inspect_sample, read_manifest, SampleState, SeedError, LEDGER_FILE_NAME, MANIFEST_FILE_NAME,
};
use pmc_domain::time::UtcTimestamp;
use pmc_platform::host_audit::{
    HostAuditLog, SAMPLE_DELETED, SAMPLE_DELETE_REJECTED, SAMPLE_RESET,
};
use pmc_platform::sample_control::{
    DeletePhase, ResetPhase, SampleControlStore, SampleOperation, SampleOutcome,
};
use pmc_platform::settings::ProtectedSettingsRoot;
use pmc_platform::workspace::{WorkspaceIdentity, WorkspaceKind};

static NEXT: AtomicU64 = AtomicU64::new(0);
const NOW: i64 = 1_790_000_000_000;

/// A protected application directory that exists only for one test.
struct TestRoot(String);

/// The same `%LOCALAPPDATA%` redirection the seed's own tests use: this
/// repo's Windows sandbox redirects writes under the real one, which trips
/// `ProtectedSettingsRoot::prepare`'s canonical-path defence for a benign
/// reason. Production is untouched.
#[cfg(windows)]
fn ensure_test_app_data_root_is_not_redirected() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("pmc-sample-test-app-data");
        fs::create_dir_all(&root).unwrap();
        let root = fs::canonicalize(&root).unwrap();
        std::env::set_var("LOCALAPPDATA", root);
    });
}

#[cfg(not(windows))]
fn ensure_test_app_data_root_is_not_redirected() {}

impl TestRoot {
    fn new(label: &str) -> Self {
        ensure_test_app_data_root_is_not_redirected();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        Self(format!("PmcSampleTest-{label}-{nonce}-{sequence}"))
    }

    fn protected_root(&self) -> ProtectedSettingsRoot {
        ProtectedSettingsRoot::prepare(&self.0).unwrap()
    }

    fn training(&self) -> WorkspaceIdentity {
        WorkspaceIdentity::resolve(&self.protected_root(), WorkspaceKind::Training).unwrap()
    }

    fn sample(&self) -> SampleWorkspace {
        SampleWorkspace::new(&self.protected_root()).unwrap()
    }

    fn folder(&self) -> PathBuf {
        self.training().root().as_path().to_path_buf()
    }

    fn vault(&self) -> PathBuf {
        self.training().synthetic_vault_root().unwrap()
    }

    fn sibling(&self, kind: &str, token: &str) -> PathBuf {
        self.folder()
            .with_file_name(format!("training.{kind}-{token}"))
    }

    fn control(&self) -> SampleControlStore {
        SampleControlStore::new(self.protected_root().path().join("sample-control-v1.json"))
    }

    fn audit_codes(&self) -> Vec<String> {
        HostAuditLog::new(self.protected_root().path().join("host-audit-v1.jsonl"))
            .events()
            .unwrap()
            .into_iter()
            .map(|event| event.event_code)
            .collect()
    }

    fn workspaces_entries(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.folder().parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn set_active(&self, operation: SampleOperation) {
        self.control()
            .update(|control| -> Result<(), ()> {
                control.active = Some(operation);
                Ok(())
            })
            .unwrap()
            .unwrap();
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        if let Ok(root) = ProtectedSettingsRoot::prepare(&self.0) {
            let _ = fs::remove_dir_all(root.path());
        }
    }
}

fn now() -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(NOW)
}

fn built(prepared: PreparedSample) {
    assert!(
        matches!(prepared, PreparedSample::Built(_)),
        "expected a build, got {prepared:?}"
    );
}

/// The digest a reset records for the sample it verified: the manifest of
/// the sample now in place.
fn installed_manifest_digest(root: &TestRoot) -> String {
    use sha2::{Digest, Sha256};
    let manifest = read_manifest(&root.folder()).unwrap().unwrap();
    Sha256::digest(serde_json::to_vec(&manifest).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn ledger_bytes(root: &TestRoot) -> Vec<u8> {
    fs::read(root.folder().join(LEDGER_FILE_NAME)).unwrap()
}

/// One root, in order, because every build runs the whole scenario.
#[test]
fn preparing_reuses_rebuilds_or_refuses_and_never_deletes_what_pmc_did_not_write() {
    let root = TestRoot::new("prepare");
    let sample = root.sample();
    assert_eq!(
        inspect_sample(&root.training()).unwrap(),
        SampleState::Absent
    );

    // Absent: built, and nothing left beside it.
    built(sample.prepare(None, "op-1", now()).unwrap());
    assert_eq!(root.workspaces_entries(), vec!["training".to_owned()]);

    // Complete and current: reused, nothing rewritten.
    let before = ledger_bytes(&root);
    assert!(matches!(
        sample
            .prepare(Some(WorkspaceKind::Live), "op-2", now())
            .unwrap(),
        PreparedSample::Reused(_)
    ));
    assert_eq!(ledger_bytes(&root), before);

    // A file a person put into the sample Vault is part of the sample
    // (product owner, 2026-09-23).
    fs::write(root.vault().join("Research").join("my-notes.md"), "mine").unwrap();
    assert!(matches!(
        sample.prepare(None, "op-3", now()).unwrap(),
        PreparedSample::Reused(_)
    ));

    // A sample file that changed: rebuilt beside it and swapped in.
    fs::write(
        root.vault().join("Research").join("market-scan.md"),
        "changed",
    )
    .unwrap();
    assert!(matches!(
        sample.inspect().unwrap(),
        SampleState::Outdated(_)
    ));
    built(sample.prepare(None, "op-4", now()).unwrap());
    assert!(matches!(sample.inspect().unwrap(), SampleState::Current(_)));
    assert_eq!(root.workspaces_entries(), vec!["training".to_owned()]);

    // An interrupted build leaves no manifest (it is written last).
    fs::remove_file(root.folder().join(MANIFEST_FILE_NAME)).unwrap();
    assert!(matches!(
        sample.inspect().unwrap(),
        SampleState::Outdated(_)
    ));
    built(sample.prepare(None, "op-5", now()).unwrap());

    // A Ledger that is not at this schema is not current, whatever the
    // manifest says.
    fs::write(root.folder().join(LEDGER_FILE_NAME), b"not a ledger").unwrap();
    assert!(matches!(
        sample.inspect().unwrap(),
        SampleState::Outdated(_)
    ));
    built(sample.prepare(None, "op-6", now()).unwrap());

    // Never while the sample is the workspace that is open.
    assert!(matches!(
        sample.prepare(Some(WorkspaceKind::Training), "op-7", now()),
        Err(SampleError::SampleIsOpen)
    ));

    // Something PMC does not write, beside its own files: refused, and
    // nothing at all is deleted.
    let stranger = root.folder().join("not-pmc.txt");
    fs::write(&stranger, "someone else's").unwrap();
    let before = ledger_bytes(&root);
    assert!(matches!(
        sample.prepare(None, "op-8", now()),
        Err(SampleError::Seed(SeedError::ForeignContents(_)))
    ));
    assert!(stranger.exists());
    assert_eq!(ledger_bytes(&root), before);

    // Live was never touched.
    assert!(!root
        .protected_root()
        .path()
        .join("workspaces")
        .join("live")
        .exists());
}

#[test]
fn another_seed_or_an_unreadable_manifest_is_never_replaced() {
    let root = TestRoot::new("foreign");
    let sample = root.sample();
    let folder = root.folder();
    fs::create_dir_all(&folder).unwrap();

    fs::write(folder.join(MANIFEST_FILE_NAME), "{ not json").unwrap();
    assert!(matches!(
        sample.prepare(None, "op-1", now()),
        Err(SampleError::Seed(SeedError::ForeignContents(_)))
    ));
    assert!(folder.join(MANIFEST_FILE_NAME).exists());

    fs::write(
        folder.join(MANIFEST_FILE_NAME),
        r#"{"seed_id":"someone-else","seed_version":1,"schema_version":48,"seed_clock_millis":0,"generator_version":"x","ledger_revision":0,"counts":{},"vault_files":[],"logical_snapshot_sha256":""}"#,
    )
    .unwrap();
    assert!(matches!(sample.inspect().unwrap(), SampleState::Foreign(_)));
    assert!(matches!(
        sample.reset(WorkspaceKind::Live, "op-2", now()),
        Err(SampleError::Seed(SeedError::ForeignContents(_)))
    ));
    assert!(folder.join(MANIFEST_FILE_NAME).exists());
}

#[test]
fn live_is_never_inspected_or_seeded() {
    let root = TestRoot::new("live");
    let live = WorkspaceIdentity::resolve(&root.protected_root(), WorkspaceKind::Live).unwrap();
    assert!(inspect_sample(&live).is_err());
    assert!(live.synthetic_seed_policy().authorize().is_err());
}

#[test]
fn a_reset_from_live_replaces_the_sample_and_is_recorded() {
    let root = TestRoot::new("reset");
    let sample = root.sample();
    built(sample.prepare(None, "op-1", now()).unwrap());
    fs::write(root.vault().join("Research").join("my-notes.md"), "mine").unwrap();

    // From inside the sample it is refused.
    assert!(matches!(
        sample.reset(WorkspaceKind::Training, "op-2", now()),
        Err(SampleError::SampleIsOpen)
    ));
    assert!(root.vault().join("Research").join("my-notes.md").exists());

    let report = sample.reset(WorkspaceKind::Live, "op-2", now()).unwrap();
    assert_eq!(report.training_root, root.folder());
    assert!(matches!(sample.inspect().unwrap(), SampleState::Current(_)));
    assert!(!root.vault().join("Research").join("my-notes.md").exists());
    assert_eq!(root.workspaces_entries(), vec!["training".to_owned()]);
    assert!(root.audit_codes().iter().any(|code| code == SAMPLE_RESET));
    // A repeated request answers with the sample it left.
    let again = sample.reset(WorkspaceKind::Live, "op-2", now()).unwrap();
    assert_eq!(again.manifest, report.manifest);
}

#[test]
fn an_interrupted_reset_rolls_back_until_the_fresh_sample_is_in_place() {
    let root = TestRoot::new("reset-crash");
    let sample = root.sample();
    built(sample.prepare(None, "op-1", now()).unwrap());
    let before = ledger_bytes(&root);

    // Building: a half-written staging folder beside an intact sample.
    let staging = root.sibling("staging", "op-2");
    fs::create_dir_all(staging.join("demo-vault")).unwrap();
    fs::write(staging.join(LEDGER_FILE_NAME), b"partial").unwrap();
    root.set_active(SampleOperation::Reset {
        operation_id: "op-2".to_owned(),
        phase: ResetPhase::Building,
        started_at_millis: NOW,
        built_manifest_sha256: None,
    });
    assert_eq!(
        sample.reconcile(now()).unwrap(),
        Some(SampleOutcome::ResetRolledBack)
    );
    assert_eq!(ledger_bytes(&root), before);
    assert_eq!(root.workspaces_entries(), vec!["training".to_owned()]);

    // After the first rename: the sample is aside, staging not yet in place.
    let retired = root.sibling("retired", "op-3");
    let staging = root.sibling("staging", "op-3");
    fs::rename(root.folder(), &retired).unwrap();
    fs::create_dir_all(&staging).unwrap();
    root.set_active(SampleOperation::Reset {
        operation_id: "op-3".to_owned(),
        phase: ResetPhase::SwappedAside,
        started_at_millis: NOW,
        built_manifest_sha256: None,
    });
    assert_eq!(
        sample.reconcile(now()).unwrap(),
        Some(SampleOutcome::ResetRolledBack)
    );
    assert_eq!(ledger_bytes(&root), before);
    assert_eq!(root.workspaces_entries(), vec!["training".to_owned()]);

    // After the second rename, before it was recorded: the reset completes.
    let retired = root.sibling("retired", "op-4");
    fs::create_dir_all(&retired).unwrap();
    root.set_active(SampleOperation::Reset {
        operation_id: "op-4".to_owned(),
        phase: ResetPhase::SwappedAside,
        started_at_millis: NOW,
        built_manifest_sha256: Some(installed_manifest_digest(&root)),
    });
    assert_eq!(sample.reconcile(now()).unwrap(), Some(SampleOutcome::Reset));
    assert_eq!(ledger_bytes(&root), before);
    assert_eq!(root.workspaces_entries(), vec!["training".to_owned()]);
}

#[test]
fn a_delete_binds_what_exists_and_removes_it_only_after_approval() {
    let root = TestRoot::new("delete");
    let sample = root.sample();
    built(sample.prepare(None, "op-1", now()).unwrap());

    // From inside the sample, and with nothing there, it is refused.
    assert!(matches!(
        sample.prepare_delete(WorkspaceKind::Training, "intent-1", 5, 60_000, now()),
        Err(SampleError::SampleIsOpen)
    ));

    // Rejected: nothing changes.
    let preview = sample
        .prepare_delete(WorkspaceKind::Live, "intent-1", 5, 60_000, now())
        .unwrap();
    assert!(preview.has_ledger && preview.has_vault && preview.has_generated_files);
    sample.reject_delete("intent-1", now()).unwrap();
    assert!(matches!(sample.inspect().unwrap(), SampleState::Current(_)));

    // Something changes after the preview: refused, nothing deleted.
    let preview = sample
        .prepare_delete(WorkspaceKind::Live, "intent-2", 5, 60_000, now())
        .unwrap();
    fs::write(root.vault().join("Research").join("my-notes.md"), "mine").unwrap();
    let approve = |intent: &'static str, payload: &str, settings: u64, id: &'static str| {
        sample.approve_delete(
            WorkspaceKind::Live,
            &ApproveSampleDelete {
                prepared_intent_id: intent,
                payload_sha256: payload,
                settings_revision: settings,
                idempotency_id: id,
            },
            now(),
        )
    };
    assert!(matches!(
        approve("intent-2", &preview.payload_sha256, 5, "approve-2"),
        Err(SampleError::Changed)
    ));
    assert!(root.folder().exists());

    // The settings moved since the preview: refused.
    let preview = sample
        .prepare_delete(WorkspaceKind::Live, "intent-3", 5, 60_000, now())
        .unwrap();
    assert!(matches!(
        approve("intent-3", &preview.payload_sha256, 6, "approve-3"),
        Err(SampleError::Changed)
    ));

    // Expired: refused.
    let preview = sample
        .prepare_delete(
            WorkspaceKind::Live,
            "intent-4",
            5,
            60_000,
            UtcTimestamp::from_unix_millis(NOW - 120_000),
        )
        .unwrap();
    assert!(matches!(
        approve("intent-4", &preview.payload_sha256, 5, "approve-4"),
        Err(SampleError::Expired)
    ));
    assert!(root.folder().exists());

    // Approved as previewed: gone, recorded, and a repeat answers the same.
    let preview = sample
        .prepare_delete(WorkspaceKind::Live, "intent-5", 5, 60_000, now())
        .unwrap();
    assert!(matches!(
        approve("intent-5", "a different payload", 5, "approve-5"),
        Err(SampleError::PayloadMismatch)
    ));
    assert_eq!(
        approve("intent-5", &preview.payload_sha256, 5, "approve-5").unwrap(),
        SampleOutcome::Deleted
    );
    assert!(!root.folder().exists());
    assert!(root.workspaces_entries().is_empty());
    assert!(root.audit_codes().iter().any(|code| code == SAMPLE_DELETED));
    assert_eq!(
        approve("intent-5", &preview.payload_sha256, 5, "approve-5").unwrap(),
        SampleOutcome::Deleted
    );
    assert!(matches!(
        sample.prepare_delete(WorkspaceKind::Live, "intent-6", 5, 60_000, now()),
        Err(SampleError::NothingToDelete)
    ));
}

#[test]
fn an_interrupted_delete_is_gone_once_renamed_aside_and_kept_before() {
    let root = TestRoot::new("delete-crash");
    let sample = root.sample();
    built(sample.prepare(None, "op-1", now()).unwrap());
    let before = ledger_bytes(&root);
    let preview = sample
        .prepare_delete(WorkspaceKind::Live, "intent-1", 5, 60_000, now())
        .unwrap();
    let control = root.control().load().unwrap();
    let Some(SampleOperation::DeletePrepared { prepared }) = control.active else {
        panic!("expected a prepared delete");
    };
    assert_eq!(prepared.payload_sha256, preview.payload_sha256);

    // Approved, but the rename never happened: not deleted.
    root.set_active(SampleOperation::Deleting {
        prepared: prepared.clone(),
        idempotency_id: "approve-1".to_owned(),
        receipt_id: "receipt-1".to_owned(),
        phase: DeletePhase::Approved,
    });
    assert_eq!(
        sample.reconcile(now()).unwrap(),
        Some(SampleOutcome::NotDeleted)
    );
    assert_eq!(ledger_bytes(&root), before);

    // Renamed aside, not yet removed: the next start finishes the removal.
    fs::rename(root.folder(), root.sibling("deleted", "approve-2")).unwrap();
    root.set_active(SampleOperation::Deleting {
        prepared,
        idempotency_id: "approve-2".to_owned(),
        receipt_id: "receipt-2".to_owned(),
        phase: DeletePhase::RenamedAside,
    });
    assert_eq!(
        sample.reconcile(now()).unwrap(),
        Some(SampleOutcome::Deleted)
    );
    assert!(root.workspaces_entries().is_empty());
}

#[test]
fn a_delete_only_prepared_at_startup_is_discarded() {
    let root = TestRoot::new("delete-prepared");
    let sample = root.sample();
    built(sample.prepare(None, "op-1", now()).unwrap());
    sample
        .prepare_delete(WorkspaceKind::Live, "intent-1", 5, 60_000, now())
        .unwrap();
    assert_eq!(sample.reconcile(now()).unwrap(), None);
    assert!(root.control().load().unwrap().active.is_none());
    assert!(matches!(sample.inspect().unwrap(), SampleState::Current(_)));
}

#[test]
fn nothing_pmc_did_not_create_beside_the_sample_is_ever_removed() {
    let root = TestRoot::new("siblings");
    let sample = root.sample();
    built(sample.prepare(None, "op-1", now()).unwrap());

    // A staging folder already there under the reset's name is refused and
    // left exactly as it was.
    let squatter = root.sibling("staging", "op-2");
    fs::create_dir_all(&squatter).unwrap();
    fs::write(squatter.join("mine.txt"), "not PMC's").unwrap();
    assert!(sample.reset(WorkspaceKind::Live, "op-2", now()).is_err());
    assert!(squatter.join("mine.txt").exists());

    // A folder merely named like PMC's own is never swept.
    let lookalike = root.sibling("deleted", "notes");
    fs::create_dir_all(&lookalike).unwrap();
    fs::write(lookalike.join("mine.txt"), "not PMC's").unwrap();
    assert_eq!(sample.reconcile(now()).unwrap(), None);
    assert!(lookalike.join("mine.txt").exists());
    assert!(squatter.join("mine.txt").exists());
    assert!(matches!(sample.inspect().unwrap(), SampleState::Current(_)));
}

#[test]
fn an_interrupted_reset_never_moves_the_previous_sample_before_the_swap() {
    let root = TestRoot::new("reset-early");
    let sample = root.sample();
    built(sample.prepare(None, "op-1", now()).unwrap());
    let before = ledger_bytes(&root);

    // Building, and the staging folder was never created: the sample in
    // place is the previous one and stays.
    root.set_active(SampleOperation::Reset {
        operation_id: "op-2".to_owned(),
        phase: ResetPhase::Building,
        started_at_millis: NOW,
        built_manifest_sha256: None,
    });
    assert_eq!(
        sample.reconcile(now()).unwrap(),
        Some(SampleOutcome::ResetRolledBack)
    );
    assert_eq!(ledger_bytes(&root), before);
    assert!(matches!(sample.inspect().unwrap(), SampleState::Current(_)));
}

#[test]
fn a_reset_in_place_that_does_not_verify_is_rolled_back_to_the_previous_sample() {
    let root = TestRoot::new("reset-unverified");
    let sample = root.sample();
    built(sample.prepare(None, "op-1", now()).unwrap());
    let before = ledger_bytes(&root);

    // The previous sample renamed aside, a broken copy in place, staging gone.
    let retired = root.sibling("retired", "op-2");
    fs::rename(root.folder(), &retired).unwrap();
    fs::create_dir_all(root.folder()).unwrap();
    fs::write(root.folder().join(LEDGER_FILE_NAME), b"broken").unwrap();
    root.set_active(SampleOperation::Reset {
        operation_id: "op-2".to_owned(),
        phase: ResetPhase::SwappedAside,
        started_at_millis: NOW,
        built_manifest_sha256: None,
    });
    assert_eq!(
        sample.reconcile(now()).unwrap(),
        Some(SampleOutcome::ResetRolledBack)
    );
    assert_eq!(ledger_bytes(&root), before);
    assert_eq!(root.workspaces_entries(), vec!["training".to_owned()]);
}

#[test]
fn a_delete_binds_folders_and_refusals_are_recorded_and_ids_do_not_cross_operations() {
    let root = TestRoot::new("delete-binding");
    let sample = root.sample();
    built(sample.prepare(None, "op-1", now()).unwrap());
    let approve = |intent: &'static str, payload: &str, id: &'static str| {
        sample.approve_delete(
            WorkspaceKind::Live,
            &ApproveSampleDelete {
                prepared_intent_id: intent,
                payload_sha256: payload,
                settings_revision: 5,
                idempotency_id: id,
            },
            now(),
        )
    };

    // An empty folder added after the preview is a change.
    let preview = sample
        .prepare_delete(WorkspaceKind::Live, "intent-1", 5, 60_000, now())
        .unwrap();
    assert_eq!(preview.effect, "irreversible_live_unaffected");
    fs::create_dir_all(root.vault().join("Empty")).unwrap();
    assert!(matches!(
        approve("intent-1", &preview.payload_sha256, "approve-1"),
        Err(SampleError::Changed)
    ));
    assert!(root.folder().exists());
    assert!(root
        .audit_codes()
        .iter()
        .any(|code| code == SAMPLE_DELETE_REJECTED));

    // An id used by a reset never answers for a delete.
    sample
        .reset(WorkspaceKind::Live, "shared-id", now())
        .unwrap();
    let preview = sample
        .prepare_delete(WorkspaceKind::Live, "intent-2", 5, 60_000, now())
        .unwrap();
    assert!(matches!(
        approve("intent-2", &preview.payload_sha256, "shared-id"),
        Err(SampleError::InvalidIdentifier)
    ));
    assert!(root.folder().exists());

    // The same approval with another payload is not the same approval.
    assert!(matches!(
        approve("intent-2", "another payload", "approve-2"),
        Err(SampleError::PayloadMismatch)
    ));
    // The same approval id for another preview is refused, not answered.
    assert_eq!(
        approve("intent-2", &preview.payload_sha256, "approve-2").unwrap(),
        SampleOutcome::Deleted
    );
    assert!(matches!(
        approve("intent-3", &preview.payload_sha256, "approve-2"),
        Err(SampleError::InvalidIdentifier)
    ));
}
