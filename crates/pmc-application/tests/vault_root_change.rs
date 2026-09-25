//! The H2b Product Vault authority-path change (item ⑦; the accepted DG3
//! Vault-root amendment §3), against a real SQLite Ledger, a real settings
//! document and real folders.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_application::vault_root_change::{
    ApproveVaultRootChange, PrepareVaultRootChange, VaultRootChangeError, VaultRootService,
};
use pmc_domain::classification::DataClassification;
use pmc_domain::evidence::{
    CreateEvidenceReference, EvidenceFingerprint, FingerprintAlgorithm, OperationContext,
    VaultRelativePath,
};
use pmc_domain::identity::{AuditEventId, CorrelationId, EvidenceReferenceId, IdempotencyId};
use pmc_domain::provenance::Provenance;
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::{EvidenceVerification, IntegrityDigest};
use pmc_ledger::sqlite::SqliteProductLedger;
use pmc_platform::authority_control::VaultRootChangeOutcome;
use pmc_platform::backup_registry::reader_sha256;
use pmc_platform::backup_registry::BackupRecord;
use pmc_platform::settings::{CanonicalDirectoryPath, ProtectedSettingsRoot, SettingsStore};
use pmc_platform::workspace::{WorkspaceIdentity, WorkspaceKind};

static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[cfg(windows)]
fn ensure_test_app_data_root_is_not_redirected() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("pmc-application-test-app-data");
        fs::create_dir_all(&root)
            .unwrap_or_else(|error| panic!("test app-data override root setup failed: {error}"));
        let root = fs::canonicalize(&root)
            .unwrap_or_else(|error| panic!("test app-data override root setup failed: {error}"));
        // SAFETY: once, before any test thread reads `LOCALAPPDATA`.
        unsafe {
            std::env::set_var("LOCALAPPDATA", root);
        }
    });
}

#[cfg(not(windows))]
fn ensure_test_app_data_root_is_not_redirected() {}

struct Fixture {
    root: ProtectedSettingsRoot,
}

impl Fixture {
    fn new(test_name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let sequence = NEXT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        ensure_test_app_data_root_is_not_redirected();
        let root = ProtectedSettingsRoot::prepare(&format!(
            "pmc-vault-root-{test_name}-{nonce}-{sequence}"
        ))
        .unwrap_or_else(|error| panic!("protected root setup failed: {error}"));
        Self { root }
    }

    fn ledger(&self) -> SqliteProductLedger {
        let identity = WorkspaceIdentity::resolve(&self.root, WorkspaceKind::Live)
            .unwrap_or_else(|error| panic!("workspace resolution failed: {error}"));
        let workspace = identity.root().as_path().to_path_buf();
        fs::create_dir_all(&workspace)
            .unwrap_or_else(|error| panic!("workspace setup failed: {error}"));
        SqliteProductLedger::open(workspace.join("ledger.sqlite3"))
            .unwrap_or_else(|error| panic!("ledger open failed: {error:?}"))
    }

    fn settings(&self) -> SettingsStore {
        let opened = SettingsStore::open(&self.root)
            .unwrap_or_else(|error| panic!("settings open failed: {error}"));
        let store = opened.into_store();
        store
            .initialize()
            .unwrap_or_else(|error| panic!("settings initialize failed: {error}"));
        store
    }

    fn service(&self, kind: WorkspaceKind) -> VaultRootService {
        VaultRootService::in_directory(self.root.path(), "live", kind)
    }

    /// A folder outside the protected root, with the given files in it.
    fn vault(&self, name: &str, files: &[(&str, &str)]) -> CanonicalDirectoryPath {
        let path = self.root.path().join(name);
        fs::create_dir_all(&path).unwrap_or_else(|error| panic!("vault setup failed: {error}"));
        for (relative, contents) in files {
            let file = path.join(relative);
            if let Some(parent) = file.parent() {
                fs::create_dir_all(parent)
                    .unwrap_or_else(|error| panic!("vault setup failed: {error}"));
            }
            fs::write(&file, contents)
                .unwrap_or_else(|error| panic!("vault setup failed: {error}"));
        }
        CanonicalDirectoryPath::new(
            fs::canonicalize(&path).unwrap_or_else(|error| panic!("canonicalize: {error}")),
        )
        .unwrap_or_else(|error| panic!("vault path: {error}"))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(self.root.path());
    }
}

fn digest_of(contents: &str) -> IntegrityDigest {
    let digest =
        reader_sha256(contents.as_bytes()).unwrap_or_else(|error| panic!("digest: {error}"));
    IntegrityDigest::parse(digest).unwrap_or_else(|error| panic!("digest: {error:?}"))
}

/// One Evidence reference, pinned to the content of `contents` unless
/// `pinned` is false.
fn seed_evidence(
    ledger: &mut SqliteProductLedger,
    token: &str,
    relative_path: &str,
    contents: &str,
    pinned: bool,
) -> EvidenceReferenceId {
    let id = EvidenceReferenceId::parse(format!("synthetic-evidence-{token}"))
        .unwrap_or_else(|error| panic!("id: {error:?}"));
    let digest = digest_of(contents);
    let fingerprint =
        pinned.then(|| EvidenceFingerprint::new(FingerprintAlgorithm::Sha256, digest.clone()));
    ledger
        .create_evidence_reference(
            CreateEvidenceReference {
                id: id.clone(),
                vault_path: VaultRelativePath::parse(relative_path)
                    .unwrap_or_else(|error| panic!("path: {error:?}")),
                fingerprint,
                verification: EvidenceVerification::Unverified,
                classification: Some(DataClassification::Internal),
                provenance: Provenance::UserEntered,
                context: OperationContext {
                    idempotency_id: IdempotencyId::parse(format!("synthetic-create-{token}"))
                        .unwrap_or_else(|error| panic!("id: {error:?}")),
                    correlation_id: CorrelationId::parse(format!("synthetic-correlation-{token}"))
                        .unwrap_or_else(|error| panic!("id: {error:?}")),
                },
            },
            AuditEventId::parse(format!("synthetic-audit-{token}"))
                .unwrap_or_else(|error| panic!("id: {error:?}")),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap_or_else(|error| panic!("seed failed: {error:?}"));
    id
}

/// A verified Operational Backup of the Ledger as it is now.
fn recovery_for(ledger: &SqliteProductLedger, destination: &Path) -> BackupRecord {
    BackupRecord {
        archive_id: "synthetic-archive-1".to_owned(),
        file_name: "pmc-operational-synthetic.tar.zst.age".to_owned(),
        destination: destination.to_path_buf(),
        created_at_millis: 5_000,
        verified_at_millis: 5_500,
        ledger_schema_version: pmc_ledger::sqlite::CURRENT_SCHEMA_VERSION,
        ledger_revision: ledger
            .revision()
            .unwrap_or_else(|error| panic!("revision: {error:?}")),
        container_sha256: "0".repeat(64),
        container_bytes: 1,
        snapshot_sha256: "1".repeat(64),
        authority_inventory_sha256: "2".repeat(64),
        authority_record_count: 0,
    }
}

fn now(millis: i64) -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(millis)
}

fn prepare_input<'a>(
    proposed: &'a CanonicalDirectoryPath,
    ledger: &'a SqliteProductLedger,
    document: &'a pmc_platform::settings::SettingsDocument,
    recovery: &'a BackupRecord,
) -> PrepareVaultRootChange<'a> {
    PrepareVaultRootChange {
        proposed_root: proposed,
        ledger,
        settings: document,
        recovery,
        recovery_not_before_millis: 5_000,
        recovery_settings_revision: document.revision,
        prepared_intent_id: "intent-1",
        confirmation_code: "AB12CD",
        valid_for_millis: 300_000,
    }
}

fn stored_folder(settings: &SettingsStore) -> Option<PathBuf> {
    settings
        .read()
        .unwrap_or_else(|error| panic!("read: {error}"))
        .operational
        .live_vault_root
        .map(|folder| folder.as_stored().to_path_buf())
}

#[test]
fn a_folder_that_serves_every_pinned_reference_is_previewed_then_committed() {
    let fixture = Fixture::new("happy");
    let mut ledger = fixture.ledger();
    seed_evidence(&mut ledger, "1", "notes/one.md", "first", true);
    seed_evidence(&mut ledger, "2", "two.md", "second", true);
    let proposed = fixture.vault(
        "new-vault",
        &[("notes/one.md", "first"), ("two.md", "second")],
    );
    let settings = fixture.settings();
    let service = fixture.service(WorkspaceKind::Live);
    let recovery = recovery_for(&ledger, fixture.root.path());
    let document = settings
        .read()
        .unwrap_or_else(|error| panic!("read: {error}"));

    let preview = service
        .prepare(
            &prepare_input(&proposed, &ledger, &document, &recovery),
            now(6_000),
        )
        .unwrap_or_else(|error| panic!("prepare failed: {error}"));
    assert_eq!(preview.evidence_count, 2);
    assert_eq!(preview.resolved_count, 2);
    assert_eq!(preview.proposed_folder_name, "new-vault");
    assert_eq!(preview.previous_folder_name, None);
    // Nothing is committed by a preview.
    assert_eq!(stored_folder(&settings), None);

    let outcome = service
        .approve(
            &ApproveVaultRootChange {
                prepared_intent_id: &preview.prepared_intent_id,
                payload_sha256: &preview.payload_sha256,
                typed_code: &preview.confirmation_code,
                idempotency_id: "approve-1",
                receipt_id: "receipt-1",
                settings: &settings,
                ledger: &ledger,
            },
            now(7_000),
        )
        .unwrap_or_else(|error| panic!("approve failed: {error}"));
    assert_eq!(outcome, VaultRootChangeOutcome::Changed);
    assert_eq!(
        stored_folder(&settings).as_deref(),
        Some(proposed.as_path())
    );
    let control = service
        .control()
        .unwrap_or_else(|error| panic!("control: {error}"));
    assert!(control.active.is_none());
    assert_eq!(control.finished.len(), 1);
    assert_eq!(control.finished[0].outcome, VaultRootChangeOutcome::Changed);
    assert_eq!(control.finished[0].folder_name, "new-vault");
}

#[test]
fn a_reference_with_no_pinned_fingerprint_blocks_the_change() {
    let fixture = Fixture::new("unpinned");
    let mut ledger = fixture.ledger();
    seed_evidence(&mut ledger, "1", "one.md", "first", true);
    seed_evidence(&mut ledger, "2", "two.md", "second", false);
    let proposed = fixture.vault("new-vault", &[("one.md", "first"), ("two.md", "second")]);
    let settings = fixture.settings();
    let service = fixture.service(WorkspaceKind::Live);
    let recovery = recovery_for(&ledger, fixture.root.path());
    let document = settings
        .read()
        .unwrap_or_else(|error| panic!("read: {error}"));

    // The file is there and holds the same bytes, but the Ledger pinned
    // nothing, so nothing can be proved: the change is refused rather than
    // assumed from the last observation.
    let error = service
        .prepare(
            &prepare_input(&proposed, &ledger, &document, &recovery),
            now(6_000),
        )
        .expect_err("an unpinned reference must block the change");
    assert!(
        matches!(error, VaultRootChangeError::UnpinnedEvidence { count: 1 }),
        "{error:?}"
    );
    assert!(service
        .control()
        .unwrap_or_else(|error| panic!("control: {error}"))
        .active
        .is_none());
}

#[test]
fn a_file_with_different_content_under_the_new_folder_blocks_the_change() {
    let fixture = Fixture::new("different");
    let mut ledger = fixture.ledger();
    seed_evidence(&mut ledger, "1", "one.md", "first", true);
    seed_evidence(&mut ledger, "2", "two.md", "second", true);
    // `two.md` is there but holds something else; `one.md` is fine.
    let proposed = fixture.vault("new-vault", &[("one.md", "first"), ("two.md", "changed")]);
    let settings = fixture.settings();
    let service = fixture.service(WorkspaceKind::Live);
    let recovery = recovery_for(&ledger, fixture.root.path());
    let document = settings
        .read()
        .unwrap_or_else(|error| panic!("read: {error}"));

    let error = service
        .prepare(
            &prepare_input(&proposed, &ledger, &document, &recovery),
            now(6_000),
        )
        .expect_err("a changed file must block the change");
    assert!(
        matches!(error, VaultRootChangeError::UnresolvedEvidence { count: 1 }),
        "{error:?}"
    );
}

#[test]
fn a_missing_file_under_the_new_folder_blocks_the_change() {
    let fixture = Fixture::new("missing");
    let mut ledger = fixture.ledger();
    seed_evidence(&mut ledger, "1", "one.md", "first", true);
    let proposed = fixture.vault("new-vault", &[]);
    let settings = fixture.settings();
    let service = fixture.service(WorkspaceKind::Live);
    let recovery = recovery_for(&ledger, fixture.root.path());
    let document = settings
        .read()
        .unwrap_or_else(|error| panic!("read: {error}"));

    let error = service
        .prepare(
            &prepare_input(&proposed, &ledger, &document, &recovery),
            now(6_000),
        )
        .expect_err("a missing file must block the change");
    assert!(
        matches!(error, VaultRootChangeError::UnresolvedEvidence { count: 1 }),
        "{error:?}"
    );
}

#[test]
fn a_backup_that_does_not_hold_the_ledger_as_it_is_now_is_not_recovery_evidence() {
    let fixture = Fixture::new("stale-backup");
    let mut ledger = fixture.ledger();
    seed_evidence(&mut ledger, "1", "one.md", "first", true);
    let proposed = fixture.vault("new-vault", &[("one.md", "first")]);
    let settings = fixture.settings();
    let service = fixture.service(WorkspaceKind::Live);
    let mut recovery = recovery_for(&ledger, fixture.root.path());
    recovery.ledger_revision -= 1;
    let document = settings
        .read()
        .unwrap_or_else(|error| panic!("read: {error}"));

    let error = service
        .prepare(
            &prepare_input(&proposed, &ledger, &document, &recovery),
            now(6_000),
        )
        .expect_err("a backup of an older Ledger is not this change's evidence");
    assert!(
        matches!(error, VaultRootChangeError::RecoveryBackupStale),
        "{error:?}"
    );
}

#[test]
fn a_second_preview_is_refused_while_one_is_prepared() {
    let fixture = Fixture::new("second-preview");
    let ledger = fixture.ledger();
    let proposed = fixture.vault("new-vault", &[]);
    let other = fixture.vault("other-vault", &[]);
    let settings = fixture.settings();
    let service = fixture.service(WorkspaceKind::Live);
    let recovery = recovery_for(&ledger, fixture.root.path());
    let document = settings
        .read()
        .unwrap_or_else(|error| panic!("read: {error}"));

    service
        .prepare(
            &prepare_input(&proposed, &ledger, &document, &recovery),
            now(6_000),
        )
        .unwrap_or_else(|error| panic!("prepare failed: {error}"));
    let mut second = prepare_input(&other, &ledger, &document, &recovery);
    second.prepared_intent_id = "intent-2";
    let error = service
        .prepare(&second, now(6_100))
        .expect_err("two changes must not be prepared at once");
    assert!(
        matches!(error, VaultRootChangeError::AnotherChangeActive),
        "{error:?}"
    );
}

#[test]
fn a_wrong_code_or_a_changed_preview_changes_nothing() {
    let fixture = Fixture::new("wrong-code");
    let ledger = fixture.ledger();
    let proposed = fixture.vault("new-vault", &[]);
    let settings = fixture.settings();
    let service = fixture.service(WorkspaceKind::Live);
    let recovery = recovery_for(&ledger, fixture.root.path());
    let document = settings
        .read()
        .unwrap_or_else(|error| panic!("read: {error}"));
    let preview = service
        .prepare(
            &prepare_input(&proposed, &ledger, &document, &recovery),
            now(6_000),
        )
        .unwrap_or_else(|error| panic!("prepare failed: {error}"));

    let approve = |code: &str, payload: &str, token: &str| {
        service.approve(
            &ApproveVaultRootChange {
                prepared_intent_id: &preview.prepared_intent_id,
                payload_sha256: payload,
                typed_code: code,
                idempotency_id: token,
                receipt_id: "receipt-1",
                settings: &settings,
                ledger: &ledger,
            },
            now(7_000),
        )
    };
    assert!(
        matches!(
            approve("ZZ99ZZ", &preview.payload_sha256, "approve-1")
                .expect_err("a wrong code must be refused"),
            VaultRootChangeError::ConfirmationMismatch
        ),
        "wrong code"
    );
    assert!(
        matches!(
            approve(&preview.confirmation_code, &"f".repeat(64), "approve-2")
                .expect_err("a different preview must be refused"),
            VaultRootChangeError::PayloadMismatch
        ),
        "wrong payload"
    );
    assert_eq!(stored_folder(&settings), None);
    // Still prepared: a refused confirmation is not a rejection.
    assert!(service
        .control()
        .unwrap_or_else(|error| panic!("control: {error}"))
        .active
        .is_some());
}

#[test]
fn an_evidence_created_between_the_preview_and_the_approval_changes_nothing() {
    let fixture = Fixture::new("ledger-moved");
    let mut ledger = fixture.ledger();
    seed_evidence(&mut ledger, "1", "one.md", "first", true);
    let proposed = fixture.vault("new-vault", &[("one.md", "first")]);
    let settings = fixture.settings();
    let service = fixture.service(WorkspaceKind::Live);
    let recovery = recovery_for(&ledger, fixture.root.path());
    let document = settings
        .read()
        .unwrap_or_else(|error| panic!("read: {error}"));
    let preview = service
        .prepare(
            &prepare_input(&proposed, &ledger, &document, &recovery),
            now(6_000),
        )
        .unwrap_or_else(|error| panic!("prepare failed: {error}"));

    // A second Evidence appears; the preview said nothing about it.
    seed_evidence(&mut ledger, "2", "two.md", "second", true);

    let error = service
        .approve(
            &ApproveVaultRootChange {
                prepared_intent_id: &preview.prepared_intent_id,
                payload_sha256: &preview.payload_sha256,
                typed_code: &preview.confirmation_code,
                idempotency_id: "approve-1",
                receipt_id: "receipt-1",
                settings: &settings,
                ledger: &ledger,
            },
            now(7_000),
        )
        .expect_err("a Ledger that moved must invalidate the preview");
    assert!(matches!(error, VaultRootChangeError::Stale), "{error:?}");
    assert_eq!(stored_folder(&settings), None);
    let control = service
        .control()
        .unwrap_or_else(|error| panic!("control: {error}"));
    assert!(control.active.is_none(), "the claim must be released");
    assert_eq!(
        control.finished[0].outcome,
        VaultRootChangeOutcome::NotChanged
    );
}

#[test]
fn a_repeated_approval_returns_the_first_outcome_and_writes_once() {
    let fixture = Fixture::new("repeat");
    let ledger = fixture.ledger();
    let proposed = fixture.vault("new-vault", &[]);
    let settings = fixture.settings();
    let service = fixture.service(WorkspaceKind::Live);
    let recovery = recovery_for(&ledger, fixture.root.path());
    let document = settings
        .read()
        .unwrap_or_else(|error| panic!("read: {error}"));
    let preview = service
        .prepare(
            &prepare_input(&proposed, &ledger, &document, &recovery),
            now(6_000),
        )
        .unwrap_or_else(|error| panic!("prepare failed: {error}"));
    let input = ApproveVaultRootChange {
        prepared_intent_id: &preview.prepared_intent_id,
        payload_sha256: &preview.payload_sha256,
        typed_code: &preview.confirmation_code,
        idempotency_id: "approve-1",
        receipt_id: "receipt-1",
        settings: &settings,
        ledger: &ledger,
    };
    let first = service
        .approve(&input, now(7_000))
        .unwrap_or_else(|error| panic!("approve failed: {error}"));
    let revision_after_first = settings
        .read()
        .unwrap_or_else(|error| panic!("read: {error}"))
        .revision;
    let again = service
        .approve(&input, now(7_100))
        .unwrap_or_else(|error| panic!("repeat failed: {error}"));

    assert_eq!(first, VaultRootChangeOutcome::Changed);
    assert_eq!(again, first);
    assert_eq!(
        settings
            .read()
            .unwrap_or_else(|error| panic!("read: {error}"))
            .revision,
        revision_after_first,
        "the settings must be written once"
    );
    assert_eq!(
        service
            .control()
            .unwrap_or_else(|error| panic!("control: {error}"))
            .finished
            .len(),
        1
    );
}

#[test]
fn rejecting_discards_the_intent_and_changes_nothing() {
    let fixture = Fixture::new("reject");
    let ledger = fixture.ledger();
    let proposed = fixture.vault("new-vault", &[]);
    let settings = fixture.settings();
    let service = fixture.service(WorkspaceKind::Live);
    let recovery = recovery_for(&ledger, fixture.root.path());
    let document = settings
        .read()
        .unwrap_or_else(|error| panic!("read: {error}"));
    let preview = service
        .prepare(
            &prepare_input(&proposed, &ledger, &document, &recovery),
            now(6_000),
        )
        .unwrap_or_else(|error| panic!("prepare failed: {error}"));

    service
        .reject(&preview.prepared_intent_id, now(6_500))
        .unwrap_or_else(|error| panic!("reject failed: {error}"));
    assert_eq!(stored_folder(&settings), None);
    assert!(service
        .control()
        .unwrap_or_else(|error| panic!("control: {error}"))
        .active
        .is_none());
    // The intent is gone: approving it afterwards finds nothing.
    let error = service
        .approve(
            &ApproveVaultRootChange {
                prepared_intent_id: &preview.prepared_intent_id,
                payload_sha256: &preview.payload_sha256,
                typed_code: &preview.confirmation_code,
                idempotency_id: "approve-1",
                receipt_id: "receipt-1",
                settings: &settings,
                ledger: &ledger,
            },
            now(7_000),
        )
        .expect_err("a rejected intent cannot be approved");
    assert!(
        matches!(error, VaultRootChangeError::NotPrepared),
        "{error:?}"
    );
}

#[test]
fn the_training_workspace_cannot_change_its_vault() {
    let fixture = Fixture::new("training");
    let ledger = fixture.ledger();
    let proposed = fixture.vault("new-vault", &[]);
    let settings = fixture.settings();
    let service = fixture.service(WorkspaceKind::Training);
    let recovery = recovery_for(&ledger, fixture.root.path());
    let document = settings
        .read()
        .unwrap_or_else(|error| panic!("read: {error}"));

    let error = service
        .prepare(
            &prepare_input(&proposed, &ledger, &document, &recovery),
            now(6_000),
        )
        .expect_err("Training derives its own Vault");
    assert!(
        matches!(error, VaultRootChangeError::NotLiveWorkspace),
        "{error:?}"
    );
}

#[test]
fn an_expired_preview_cannot_be_approved() {
    let fixture = Fixture::new("expired");
    let ledger = fixture.ledger();
    let proposed = fixture.vault("new-vault", &[]);
    let settings = fixture.settings();
    let service = fixture.service(WorkspaceKind::Live);
    let recovery = recovery_for(&ledger, fixture.root.path());
    let document = settings
        .read()
        .unwrap_or_else(|error| panic!("read: {error}"));
    let preview = service
        .prepare(
            &prepare_input(&proposed, &ledger, &document, &recovery),
            now(6_000),
        )
        .unwrap_or_else(|error| panic!("prepare failed: {error}"));

    let error = service
        .approve(
            &ApproveVaultRootChange {
                prepared_intent_id: &preview.prepared_intent_id,
                payload_sha256: &preview.payload_sha256,
                typed_code: &preview.confirmation_code,
                idempotency_id: "approve-1",
                receipt_id: "receipt-1",
                settings: &settings,
                ledger: &ledger,
            },
            now(preview.expires_at_millis + 1),
        )
        .expect_err("an expired preview must be refused");
    assert!(matches!(error, VaultRootChangeError::Expired), "{error:?}");
    assert_eq!(stored_folder(&settings), None);
}

#[test]
fn a_crash_after_the_approval_is_resolved_by_reading_the_settings_not_by_assuming() {
    let fixture = Fixture::new("reconcile");
    let ledger = fixture.ledger();
    let proposed = fixture.vault("new-vault", &[]);
    let settings = fixture.settings();
    let service = fixture.service(WorkspaceKind::Live);
    let recovery = recovery_for(&ledger, fixture.root.path());
    let document = settings
        .read()
        .unwrap_or_else(|error| panic!("read: {error}"));
    let preview = service
        .prepare(
            &prepare_input(&proposed, &ledger, &document, &recovery),
            now(6_000),
        )
        .unwrap_or_else(|error| panic!("prepare failed: {error}"));
    // Claim it and stop there, as a crash between the approval and the
    // settings write would: the record says Executing and the settings still
    // hold nothing.
    let _ = service.approve(
        &ApproveVaultRootChange {
            prepared_intent_id: &preview.prepared_intent_id,
            payload_sha256: &preview.payload_sha256,
            typed_code: "ZZ99ZZ",
            idempotency_id: "approve-1",
            receipt_id: "receipt-1",
            settings: &settings,
            ledger: &ledger,
        },
        now(7_000),
    );

    // A fresh start with nothing written: the change did not happen.
    let outcome = service
        .reconcile(&settings, now(8_000))
        .unwrap_or_else(|error| panic!("reconcile failed: {error}"));
    assert_eq!(
        outcome, None,
        "a prepared change is discarded, not finished"
    );
    assert!(service
        .control()
        .unwrap_or_else(|error| panic!("control: {error}"))
        .active
        .is_none());
    assert_eq!(stored_folder(&settings), None);
}

#[test]
fn a_backup_made_before_this_change_began_is_not_its_recovery_evidence() {
    // §3.2's evidence holds the current Ledger *and* the current settings.
    // The registry records a backup's Ledger revision but not the settings
    // it carried, so an older backup with the same Ledger revision is
    // refused on its age: it may hold settings that are not the ones being
    // replaced.
    let fixture = Fixture::new("older-backup");
    let ledger = fixture.ledger();
    let proposed = fixture.vault("new-vault", &[]);
    let settings = fixture.settings();
    let service = fixture.service(WorkspaceKind::Live);
    let recovery = recovery_for(&ledger, fixture.root.path());
    let document = settings
        .read()
        .unwrap_or_else(|error| panic!("read: {error}"));
    let mut input = prepare_input(&proposed, &ledger, &document, &recovery);
    input.recovery_not_before_millis = recovery.verified_at_millis + 1;

    let error = service
        .prepare(&input, now(6_000))
        .expect_err("a backup from before the change is not its evidence");
    assert!(
        matches!(error, VaultRootChangeError::RecoveryBackupStale),
        "{error:?}"
    );
}

#[test]
fn a_pinned_reference_that_disappears_between_the_preview_and_the_approval_stops_the_change() {
    // The approval re-resolves every file: the preview is a claim about the
    // proposed folder, not a permit for it.
    let fixture = Fixture::new("vanished");
    let mut ledger = fixture.ledger();
    seed_evidence(&mut ledger, "1", "one.md", "first", true);
    let proposed = fixture.vault("new-vault", &[("one.md", "first")]);
    let settings = fixture.settings();
    let service = fixture.service(WorkspaceKind::Live);
    let recovery = recovery_for(&ledger, fixture.root.path());
    let document = settings
        .read()
        .unwrap_or_else(|error| panic!("read: {error}"));
    let preview = service
        .prepare(
            &prepare_input(&proposed, &ledger, &document, &recovery),
            now(6_000),
        )
        .unwrap_or_else(|error| panic!("prepare failed: {error}"));

    fs::remove_file(proposed.as_path().join("one.md"))
        .unwrap_or_else(|error| panic!("remove failed: {error}"));

    let error = service
        .approve(
            &ApproveVaultRootChange {
                prepared_intent_id: &preview.prepared_intent_id,
                payload_sha256: &preview.payload_sha256,
                typed_code: &preview.confirmation_code,
                idempotency_id: "approve-1",
                receipt_id: "receipt-1",
                settings: &settings,
                ledger: &ledger,
            },
            now(7_000),
        )
        .expect_err("a file that vanished must stop the change");
    assert!(
        matches!(error, VaultRootChangeError::UnresolvedEvidence { count: 1 }),
        "{error:?}"
    );
    assert_eq!(stored_folder(&settings), None);
}

#[test]
fn a_settings_change_between_the_preview_and_the_approval_changes_nothing() {
    let fixture = Fixture::new("settings-moved");
    let ledger = fixture.ledger();
    let proposed = fixture.vault("new-vault", &[]);
    let settings = fixture.settings();
    let service = fixture.service(WorkspaceKind::Live);
    let recovery = recovery_for(&ledger, fixture.root.path());
    let document = settings
        .read()
        .unwrap_or_else(|error| panic!("read: {error}"));
    let preview = service
        .prepare(
            &prepare_input(&proposed, &ledger, &document, &recovery),
            now(6_000),
        )
        .unwrap_or_else(|error| panic!("prepare failed: {error}"));

    // Something else saves a setting; the preview's snapshot is now old.
    settings
        .apply_durable(
            document.revision,
            pmc_platform::settings::SettingsPatch::Operational(
                pmc_platform::settings::OperationalPatch {
                    retention_days: Some(45),
                    ..pmc_platform::settings::OperationalPatch::default()
                },
            ),
        )
        .unwrap_or_else(|error| panic!("commit failed: {error}"));

    let error = service
        .approve(
            &ApproveVaultRootChange {
                prepared_intent_id: &preview.prepared_intent_id,
                payload_sha256: &preview.payload_sha256,
                typed_code: &preview.confirmation_code,
                idempotency_id: "approve-1",
                receipt_id: "receipt-1",
                settings: &settings,
                ledger: &ledger,
            },
            now(7_000),
        )
        .expect_err("a settings change must invalidate the preview");
    assert!(matches!(error, VaultRootChangeError::Stale), "{error:?}");
    assert_eq!(stored_folder(&settings), None);
    assert_eq!(
        settings
            .read()
            .unwrap_or_else(|error| panic!("read: {error}"))
            .operational
            .retention_days,
        45,
        "the other change must stand"
    );
}

#[test]
fn a_backup_whose_settings_are_older_than_the_ones_being_replaced_is_not_evidence() {
    // Review finding (⑦-2b): the backup reads the settings before it takes
    // the gate, so a setting saved in between would be replaced with no
    // backup holding it. The run reports the revision its settings.json was
    // read at; the preview refuses a backup whose revision is not the one it
    // reads.
    let fixture = Fixture::new("stale-settings-backup");
    let ledger = fixture.ledger();
    let proposed = fixture.vault("new-vault", &[]);
    let settings = fixture.settings();
    let service = fixture.service(WorkspaceKind::Live);
    let recovery = recovery_for(&ledger, fixture.root.path());
    let document = settings
        .read()
        .unwrap_or_else(|error| panic!("read: {error}"));
    let mut input = prepare_input(&proposed, &ledger, &document, &recovery);
    input.recovery_settings_revision = document.revision + 1;

    let error = service
        .prepare(&input, now(6_000))
        .expect_err("a backup of other settings is not this change's evidence");
    assert!(
        matches!(error, VaultRootChangeError::RecoveryBackupStale),
        "{error:?}"
    );
    assert!(service
        .control()
        .unwrap_or_else(|error| panic!("control: {error}"))
        .active
        .is_none());
}
