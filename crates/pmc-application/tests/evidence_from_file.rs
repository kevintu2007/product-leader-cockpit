//! Evidence from a file (item ⑦-3; DG3 Vault-root and Evidence-from-file
//! amendment §4), against a real SQLite Ledger and real files in a real
//! Vault folder.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_application::desktop_runtime::OpaqueIdSource;
use pmc_application::evidence_from_file::{
    create_evidence_from_file, observe_chosen_file, preview_chosen_file, ChosenEvidenceFile,
    EvidenceFromFileError,
};
use pmc_application::evidence_writes::{DesktopVault, VaultUnavailable};
use pmc_domain::classification::DataClassification;
use pmc_domain::evidence::OperationContext;
use pmc_domain::identity::{CorrelationId, IdempotencyId};
use pmc_domain::provenance::Provenance;
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::EvidenceVerification;
use pmc_ledger::sqlite::SqliteProductLedger;
use pmc_platform::backup_registry::reader_sha256;
use pmc_platform::settings::{CanonicalDirectoryPath, DeferredDirectoryPath};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A stand-in for Windows' name comparison (the real one is tested where it
/// lives, in `pmc-platform`).
fn ascii_case_insensitive(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn scratch(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let directory =
        std::env::temp_dir().join(format!("pmc-evidence-file-{name}-{nonce}-{sequence}"));
    fs::create_dir_all(&directory).unwrap_or_else(|error| panic!("scratch: {error}"));
    fs::canonicalize(&directory).unwrap_or_else(|error| panic!("canonical: {error}"))
}

fn vault_at(root: &Path) -> DesktopVault {
    let folder = CanonicalDirectoryPath::new(root.to_path_buf())
        .unwrap_or_else(|error| panic!("vault folder: {error}"));
    DesktopVault::configured(Some(&DeferredDirectoryPath::from(folder)))
}

fn write(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|error| panic!("parent: {error}"));
    }
    fs::write(path, bytes).unwrap_or_else(|error| panic!("file: {error}"));
}

fn sha256(bytes: &[u8]) -> String {
    reader_sha256(bytes).unwrap_or_else(|error| panic!("sha256: {error}"))
}

fn ledger(directory: &Path) -> SqliteProductLedger {
    SqliteProductLedger::open(directory.join("ledger.sqlite3"))
        .unwrap_or_else(|error| panic!("ledger: {error:?}"))
}

fn context(request: &str) -> OperationContext {
    OperationContext {
        idempotency_id: IdempotencyId::parse(request).unwrap_or_else(|_| panic!("id")),
        correlation_id: CorrelationId::parse(format!("corr-{request}"))
            .unwrap_or_else(|_| panic!("id")),
    }
}

fn at(millis: i64) -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(millis)
}

fn choose(vault: &DesktopVault, file: &Path, millis: i64) -> ChosenEvidenceFile {
    observe_chosen_file(vault, file, at(millis)).unwrap_or_else(|error| panic!("{error:?}"))
}

#[test]
fn a_chosen_file_becomes_a_pinned_verified_reference_and_a_retry_creates_nothing_new() {
    let root = scratch("create");
    let vault = vault_at(&root);
    let mut ledger = ledger(&scratch("create-ledger"));
    let mut ids = OpaqueIdSource::new();
    let bytes = b"Q3 board minutes";
    let file = root.join("Board").join("第三季會議紀錄.md");
    write(&file, bytes);

    let chosen = choose(&vault, &file, 1_000);
    assert_eq!(chosen.file_name(), "第三季會議紀錄.md");
    assert_eq!(chosen.vault_path().as_str(), "Board/第三季會議紀錄.md");
    assert_eq!(
        chosen.observation().fingerprint.digest().as_str(),
        sha256(bytes)
    );
    let preview = preview_chosen_file(&ledger, &chosen, ascii_case_insensitive)
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(preview.existing, None);
    assert!(preview.same_content.is_empty());

    let created = create_evidence_from_file(
        &mut ledger,
        &vault,
        &chosen,
        DataClassification::Confidential,
        context("sheet-1"),
        &mut ids,
        at(2_000),
        ascii_case_insensitive,
    )
    .unwrap_or_else(|error| panic!("{error:?}"));
    let record = created;
    assert_eq!(record.vault_path.as_str(), "Board/第三季會議紀錄.md");
    assert_eq!(record.classification, DataClassification::Confidential);
    assert_eq!(record.provenance, Provenance::UserEntered);
    // Pinned and verified from the observation taken at create time.
    assert_eq!(
        record
            .fingerprint
            .as_ref()
            .map(|pin| pin.digest().as_str().to_owned()),
        Some(sha256(bytes))
    );
    assert!(matches!(
        &record.verification,
        EvidenceVerification::Verified { verified_at, integrity_digest }
            if *verified_at == at(2_000) && integrity_digest.as_str() == sha256(bytes)
    ));

    // The same request again (a retry after a lost answer) — even after the
    // file changed: the record the first attempt created, nothing new.
    write(&file, b"edited after the create");
    let revision = ledger
        .revision()
        .unwrap_or_else(|error| panic!("{error:?}"));
    let again = create_evidence_from_file(
        &mut ledger,
        &vault,
        &chosen,
        DataClassification::Confidential,
        context("sheet-1"),
        &mut ids,
        at(3_000),
        ascii_case_insensitive,
    )
    .unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(again, record);
    assert_eq!(
        ledger
            .revision()
            .unwrap_or_else(|error| panic!("{error:?}")),
        revision
    );
}

#[test]
fn a_file_outside_the_vault_or_no_vault_at_all_is_refused() {
    let root = scratch("outside");
    let elsewhere = scratch("outside-elsewhere");
    let file = elsewhere.join("note.md");
    write(&file, b"not in the Vault");
    assert!(matches!(
        observe_chosen_file(&vault_at(&root), &file, at(1)),
        Err(EvidenceFromFileError::OutsideVault)
    ));
    assert!(matches!(
        observe_chosen_file(&DesktopVault::configured(None), &file, at(1)),
        Err(EvidenceFromFileError::Vault(
            VaultUnavailable::NotConfigured
        ))
    ));
}

#[test]
fn changed_bytes_stop_the_create_and_the_new_observation_can_be_confirmed() {
    let root = scratch("changed");
    let vault = vault_at(&root);
    let mut ledger = ledger(&scratch("changed-ledger"));
    let mut ids = OpaqueIdSource::new();
    let file = root.join("plan.md");
    write(&file, b"first draft");
    let chosen = choose(&vault, &file, 1_000);
    write(&file, b"second draft");
    let revision = ledger
        .revision()
        .unwrap_or_else(|error| panic!("{error:?}"));

    let refreshed = match create_evidence_from_file(
        &mut ledger,
        &vault,
        &chosen,
        DataClassification::Internal,
        context("sheet-2"),
        &mut ids,
        at(2_000),
        ascii_case_insensitive,
    ) {
        Err(EvidenceFromFileError::FileChanged(refreshed)) => refreshed,
        other => panic!("expected FileChanged, got {other:?}"),
    };
    // Only the id was reserved (it is this request's, reused on confirm);
    // no reference was written.
    assert_eq!(
        ledger
            .revision()
            .unwrap_or_else(|error| panic!("{error:?}")),
        revision + 1
    );
    assert_eq!(
        refreshed.observation().fingerprint.digest().as_str(),
        sha256(b"second draft")
    );

    // The person confirms what the sheet now shows.
    let created = create_evidence_from_file(
        &mut ledger,
        &vault,
        &refreshed,
        DataClassification::Internal,
        context("sheet-2"),
        &mut ids,
        at(3_000),
        ascii_case_insensitive,
    )
    .unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(
        created
            .fingerprint
            .as_ref()
            .map(|pin| pin.digest().as_str().to_owned()),
        Some(sha256(b"second draft"))
    );
}

#[test]
fn a_file_that_already_has_a_reference_offers_that_one_and_same_content_warns() {
    let root = scratch("existing");
    let vault = vault_at(&root);
    let mut ledger = ledger(&scratch("existing-ledger"));
    let mut ids = OpaqueIdSource::new();
    let file = root.join("Reports").join("Q3.md");
    write(&file, b"quarterly report");
    let first = create_evidence_from_file(
        &mut ledger,
        &vault,
        &choose(&vault, &file, 1_000),
        DataClassification::Internal,
        context("sheet-3a"),
        &mut ids,
        at(1_500),
        ascii_case_insensitive,
    )
    .unwrap_or_else(|error| panic!("{error:?}"));

    // The same file chosen again, spelled differently.
    let again = choose(&vault, &root.join("reports").join("q3.MD"), 2_000);
    let preview = preview_chosen_file(&ledger, &again, ascii_case_insensitive)
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(
        preview.existing.as_ref().map(|found| found.id.clone()),
        Some(first.id.clone())
    );
    // The existing reference is not also listed as "same content elsewhere".
    assert!(preview.same_content.is_empty());
    let revision = ledger
        .revision()
        .unwrap_or_else(|error| panic!("{error:?}"));
    match create_evidence_from_file(
        &mut ledger,
        &vault,
        &again,
        DataClassification::Internal,
        context("sheet-3b"),
        &mut ids,
        at(2_500),
        ascii_case_insensitive,
    ) {
        Err(EvidenceFromFileError::AlreadyReferenced(found)) => assert_eq!(found.id, first.id),
        other => panic!("expected AlreadyReferenced, got {other:?}"),
    }
    // Only the reservation moved the revision; no second reference exists.
    assert_eq!(
        ledger
            .revision()
            .unwrap_or_else(|error| panic!("{error:?}")),
        revision + 1
    );

    // The same bytes under another name: a warning, not a refusal.
    let copy = root.join("Archive").join("Q3 copy.md");
    write(&copy, b"quarterly report");
    let copy_chosen = choose(&vault, &copy, 3_000);
    let preview = preview_chosen_file(&ledger, &copy_chosen, ascii_case_insensitive)
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(preview.existing, None);
    assert_eq!(
        preview
            .same_content
            .iter()
            .map(|found| found.id.clone())
            .collect::<Vec<_>>(),
        vec![first.id]
    );
    create_evidence_from_file(
        &mut ledger,
        &vault,
        &copy_chosen,
        DataClassification::Internal,
        context("sheet-3c"),
        &mut ids,
        at(3_500),
        ascii_case_insensitive,
    )
    .unwrap_or_else(|error| panic!("{error:?}"));
}

#[test]
fn a_file_chosen_under_another_vault_folder_is_not_read_under_the_new_one() {
    let first_root = scratch("moved-a");
    let second_root = scratch("moved-b");
    let mut ledger = ledger(&scratch("moved-ledger"));
    let mut ids = OpaqueIdSource::new();
    write(&first_root.join("note.md"), b"same name, first folder");
    write(&second_root.join("note.md"), b"same name, second folder");
    let chosen = choose(&vault_at(&first_root), &first_root.join("note.md"), 1_000);
    let revision = ledger
        .revision()
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert!(matches!(
        create_evidence_from_file(
            &mut ledger,
            &vault_at(&second_root),
            &chosen,
            DataClassification::Internal,
            context("sheet-4"),
            &mut ids,
            at(2_000),
            ascii_case_insensitive,
        ),
        Err(EvidenceFromFileError::VaultChanged)
    ));
    assert_eq!(
        ledger
            .revision()
            .unwrap_or_else(|error| panic!("{error:?}")),
        revision
    );
}
