//! What the Evidence-write facade adds over `knowledge.rs`: which Vault,
//! validated when, one error type, and an honest report of whether anything
//! was written. The underlying observe-and-persist behaviour is covered by
//! `tests/knowledge.rs` and is not repeated here.

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_application::evidence_writes::{
    link_to_product, pin_fingerprint, reobserve, DesktopVault, EvidenceWriteError, ReobserveResult,
    VaultUnavailable,
};
use pmc_domain::{
    audit::AuditEventIdSource,
    classification::DataClassification,
    error::ErrorCode,
    evidence::{CreateEvidenceReference, EvidenceFingerprint, FingerprintAlgorithm},
    evidence::{OperationContext, VaultRelativePath},
    identity::ProductId,
    identity::{AggregateVersion, AuditEventId, CorrelationId, EvidenceReferenceId, IdempotencyId},
    portfolio::{CreateProduct, LongText, OperationContext as PortfolioContext, ShortText},
    provenance::Provenance,
    time::UtcTimestamp,
    work_management::{EvidenceVerification, IntegrityDigest},
    DomainValueError,
};
use pmc_ledger::sqlite::{LedgerTransactionError, SqliteProductLedger};
use pmc_platform::settings::ProtectedSettingsRoot;
use pmc_platform::workspace::{WorkspaceIdentity, WorkspaceKind};

static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Windows can redirect a test process's `%LOCALAPPDATA%` into an
/// app-package container, which trips `ProtectedSettingsRoot::prepare`'s
/// "outside OS application data" validation for a reason that has nothing to
/// do with what is under test. Point it at a plain directory under `target/`
/// instead -- the same guard, for the same reason, that
/// `pmc-platform/tests/workspace_identity.rs`,
/// `pmc-platform/tests/settings_store.rs` and `pmc-seed/tests/seed.rs`
/// already use.
#[cfg(windows)]
fn ensure_test_app_data_root_is_not_redirected() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("pmc-application-test-app-data");
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

/// Deterministic ids, so a test asserting "nothing was written" is not also
/// depending on the host's real id source.
struct TestIds(u64);

impl AuditEventIdSource for TestIds {
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        self.0 += 1;
        AuditEventId::parse(format!("synthetic-audit-{}", self.0))
    }
}

/// A uniquely named protected root under the real app-data directory, the
/// same isolation `pmc-platform`'s own workspace tests use.
struct Fixture {
    root: ProtectedSettingsRoot,
    name: String,
}

impl Fixture {
    fn new(test_name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let sequence = NEXT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        ensure_test_app_data_root_is_not_redirected();
        let name = format!("pmc-evidence-writes-{test_name}-{nonce}-{sequence}");
        let root = ProtectedSettingsRoot::prepare(&name)
            .unwrap_or_else(|error| panic!("protected root setup failed: {error}"));
        Self { root, name }
    }

    fn identity(&self, kind: WorkspaceKind) -> WorkspaceIdentity {
        WorkspaceIdentity::resolve(&self.root, kind)
            .unwrap_or_else(|error| panic!("workspace resolution failed: {error}"))
    }

    /// The workspace root, created (resolution deliberately does not create).
    fn workspace_root(&self, identity: &WorkspaceIdentity) -> PathBuf {
        let root = identity.root().as_path().to_path_buf();
        fs::create_dir_all(&root).unwrap_or_else(|error| panic!("workspace setup failed: {error}"));
        root
    }

    fn ledger(&self, workspace_root: &std::path::Path) -> SqliteProductLedger {
        SqliteProductLedger::open(workspace_root.join("ledger.sqlite3"))
            .unwrap_or_else(|error| panic!("ledger open failed: {error:?}"))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(self.root.path());
        let _ = &self.name;
    }
}

fn seed_evidence(
    writer: &mut SqliteProductLedger,
    relative_path: &str,
    fingerprint: Option<EvidenceFingerprint>,
    verification: EvidenceVerification,
) -> EvidenceReferenceId {
    let id = EvidenceReferenceId::parse("synthetic-evidence-1")
        .unwrap_or_else(|error| panic!("id: {error:?}"));
    writer
        .create_evidence_reference(
            CreateEvidenceReference {
                id: id.clone(),
                vault_path: VaultRelativePath::parse(relative_path)
                    .unwrap_or_else(|error| panic!("path: {error:?}")),
                fingerprint,
                verification,
                classification: Some(DataClassification::Internal),
                provenance: Provenance::UserEntered,
                context: OperationContext {
                    idempotency_id: IdempotencyId::parse("synthetic-evidence-create-1")
                        .unwrap_or_else(|error| panic!("id: {error:?}")),
                    correlation_id: CorrelationId::parse("synthetic-evidence-correlation-1")
                        .unwrap_or_else(|error| panic!("id: {error:?}")),
                },
            },
            AuditEventId::parse("synthetic-evidence-audit-1")
                .unwrap_or_else(|error| panic!("id: {error:?}")),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap_or_else(|error| panic!("seed failed: {error:?}"));
    id
}

fn idempotency(token: &str) -> IdempotencyId {
    IdempotencyId::parse(token).unwrap_or_else(|error| panic!("id: {error:?}"))
}

fn correlation(token: &str) -> CorrelationId {
    CorrelationId::parse(token).unwrap_or_else(|error| panic!("id: {error:?}"))
}

/// Live has no Vault the application may derive, and this is reported as its
/// own condition rather than as a missing directory. Deriving a Live path by
/// convention would contradict the Product Vault design, which makes that root
/// user-selected.
#[test]
fn live_has_no_vault_to_read_and_says_so_rather_than_reporting_one_as_missing() {
    let fixture = Fixture::new("live");
    let identity = fixture.identity(WorkspaceKind::Live);
    let workspace_root = fixture.workspace_root(&identity);
    let mut ledger = fixture.ledger(&workspace_root);
    let id = seed_evidence(
        &mut ledger,
        "notes.md",
        None,
        EvidenceVerification::Unverified,
    );
    let vault = DesktopVault::resolve(&identity);
    let mut ids = TestIds(0);

    let error = pin_fingerprint(
        &mut ledger,
        &vault,
        &id,
        AggregateVersion::initial(),
        idempotency("synthetic-pin-1"),
        correlation("synthetic-pin-correlation-1"),
        &mut ids,
        UtcTimestamp::from_unix_millis(2_000),
    )
    .expect_err("Live must not resolve a Vault");
    assert!(matches!(
        error,
        EvidenceWriteError::Vault(VaultUnavailable::NotConfigured)
    ));
    // Refused before anything was minted or written.
    assert_eq!(ids.0, 0);
}

/// The design claim this test exists for: the candidate is validated on each
/// use, not once at startup. The *same* `DesktopVault` value refuses while
/// the Vault is absent and succeeds once it appears -- which is what happens
/// when a person runs `pmc-seed` while the application is open.
#[test]
fn the_same_vault_handle_refuses_before_the_vault_exists_and_works_after_it_appears() {
    let fixture = Fixture::new("appears");
    let identity = fixture.identity(WorkspaceKind::Training);
    let workspace_root = fixture.workspace_root(&identity);
    let mut ledger = fixture.ledger(&workspace_root);
    let id = seed_evidence(
        &mut ledger,
        "notes.md",
        None,
        EvidenceVerification::Unverified,
    );
    let vault = DesktopVault::resolve(&identity);
    let mut ids = TestIds(0);

    let error = reobserve(
        &mut ledger,
        &vault,
        &id,
        AggregateVersion::initial(),
        idempotency("synthetic-reobserve-1"),
        correlation("synthetic-reobserve-correlation-1"),
        &mut ids,
        UtcTimestamp::from_unix_millis(2_000),
    )
    .expect_err("an unseeded workspace has no usable Vault root");
    assert!(matches!(
        error,
        EvidenceWriteError::Vault(VaultUnavailable::InvalidRoot)
    ));

    // The seed runs. Nothing is reconstructed and the application is not
    // restarted: the same handle is used again.
    let vault_dir = identity
        .synthetic_vault_root()
        .unwrap_or_else(|| panic!("Training must derive a Vault root"));
    fs::create_dir_all(&vault_dir).unwrap_or_else(|error| panic!("vault setup failed: {error}"));
    fs::write(vault_dir.join("notes.md"), b"synthetic content")
        .unwrap_or_else(|error| panic!("note setup failed: {error}"));

    let result = reobserve(
        &mut ledger,
        &vault,
        &id,
        AggregateVersion::initial(),
        idempotency("synthetic-reobserve-2"),
        correlation("synthetic-reobserve-correlation-2"),
        &mut ids,
        UtcTimestamp::from_unix_millis(3_000),
    )
    .unwrap_or_else(|error| panic!("reobserve after seeding failed: {error:?}"));
    let ReobserveResult::Persisted(outcome) = result else {
        panic!("an unverified reference over a readable file must change");
    };
    assert!(matches!(
        outcome.record.verification,
        EvidenceVerification::ObservedUnpinned { .. }
    ));
}

/// `Unchanged` is a real outcome, not a quiet success, and the facade must
/// pass it through rather than reporting every call as a write.
///
/// Getting one requires care: a successful re-verification always refreshes
/// `verified_at`, and `ObservedUnpinned` carries `observed_at`, so
/// re-observing a *readable* file always differs from what was stored. The
/// genuine no-op is a file that is already `DegradedLastVerified` and is
/// still missing on the next check. The Ledger revision is the proof: it
/// must not move.
#[test]
fn a_reobservation_that_finds_no_change_passes_unchanged_through_and_writes_nothing() {
    let fixture = Fixture::new("unchanged");
    let identity = fixture.identity(WorkspaceKind::Training);
    let workspace_root = fixture.workspace_root(&identity);
    let vault_dir = identity
        .synthetic_vault_root()
        .unwrap_or_else(|| panic!("Training must derive a Vault root"));
    fs::create_dir_all(&vault_dir).unwrap_or_else(|error| panic!("vault setup failed: {error}"));
    let note = vault_dir.join("notes.md");
    fs::write(&note, b"here for now").unwrap_or_else(|error| panic!("note setup failed: {error}"));
    let digest = pmc_platform::filesystem::compute_sha256_fingerprint(&note)
        .map(|digest| {
            IntegrityDigest::parse(digest).unwrap_or_else(|error| panic!("digest: {error:?}"))
        })
        .unwrap_or_else(|error| panic!("digest: {error}"));

    let mut ledger = fixture.ledger(&workspace_root);
    let id = seed_evidence(
        &mut ledger,
        "notes.md",
        Some(EvidenceFingerprint::new(
            FingerprintAlgorithm::Sha256,
            digest.clone(),
        )),
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(500),
            integrity_digest: digest,
        },
    );
    let vault = DesktopVault::resolve(&identity);
    let mut ids = TestIds(0);
    fs::remove_file(&note).unwrap_or_else(|error| panic!("note removal failed: {error}"));

    // Verified -> DegradedLastVerified: a real change, so it persists.
    let ReobserveResult::Persisted(first) = reobserve(
        &mut ledger,
        &vault,
        &id,
        AggregateVersion::initial(),
        idempotency("synthetic-reobserve-1"),
        correlation("synthetic-reobserve-correlation-1"),
        &mut ids,
        UtcTimestamp::from_unix_millis(2_000),
    )
    .unwrap_or_else(|error| panic!("first reobserve failed: {error:?}")) else {
        panic!("a missing file must degrade a Verified reference");
    };
    assert!(matches!(
        first.record.verification,
        EvidenceVerification::DegradedLastVerified { .. }
    ));
    let revision_after_first = ledger
        .revision()
        .unwrap_or_else(|error| panic!("revision read failed: {error:?}"));

    // Still missing, already degraded: nothing to write.
    let ReobserveResult::Unchanged(record) = reobserve(
        &mut ledger,
        &vault,
        &id,
        first.record.version,
        idempotency("synthetic-reobserve-2"),
        correlation("synthetic-reobserve-correlation-2"),
        &mut ids,
        UtcTimestamp::from_unix_millis(3_000),
    )
    .unwrap_or_else(|error| panic!("second reobserve failed: {error:?}")) else {
        panic!("a still-missing, already-degraded reference must not persist again");
    };
    assert_eq!(record.verification, first.record.verification);
    assert_eq!(record.version, first.record.version);
    assert_eq!(
        ledger
            .revision()
            .unwrap_or_else(|error| panic!("revision read failed: {error:?}")),
        revision_after_first
    );
}

/// The three underlying error enums fold into one the caller can map, and
/// the refusal still happens before any write.
#[test]
fn an_already_pinned_reference_is_refused_through_the_single_facade_error() {
    let fixture = Fixture::new("pinned");
    let identity = fixture.identity(WorkspaceKind::Training);
    let workspace_root = fixture.workspace_root(&identity);
    let vault_dir = identity
        .synthetic_vault_root()
        .unwrap_or_else(|| panic!("Training must derive a Vault root"));
    fs::create_dir_all(&vault_dir).unwrap_or_else(|error| panic!("vault setup failed: {error}"));
    fs::write(vault_dir.join("notes.md"), b"synthetic content")
        .unwrap_or_else(|error| panic!("note setup failed: {error}"));
    let mut ledger = fixture.ledger(&workspace_root);
    let digest =
        IntegrityDigest::parse("a".repeat(64)).unwrap_or_else(|error| panic!("digest: {error:?}"));
    let id = seed_evidence(
        &mut ledger,
        "notes.md",
        Some(EvidenceFingerprint::new(
            FingerprintAlgorithm::Sha256,
            digest.clone(),
        )),
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(1_000),
            integrity_digest: digest,
        },
    );
    let vault = DesktopVault::resolve(&identity);
    let mut ids = TestIds(0);

    let error = pin_fingerprint(
        &mut ledger,
        &vault,
        &id,
        AggregateVersion::initial(),
        idempotency("synthetic-pin-1"),
        correlation("synthetic-pin-correlation-1"),
        &mut ids,
        UtcTimestamp::from_unix_millis(2_000),
    )
    .expect_err("a pin is the identity and is never rewritten");
    assert!(matches!(error, EvidenceWriteError::AlreadyPinned));
}

/// The gap the review found: the Ledger's own version check catches a change
/// made *after* the flow's read, but not a view that was already stale
/// *before* it. A click from a stale O01 view must be refused before the
/// filesystem is touched, and must write nothing.
#[test]
fn a_stale_expected_version_is_refused_before_any_observation_and_writes_nothing() {
    let fixture = Fixture::new("stale");
    let identity = fixture.identity(WorkspaceKind::Training);
    let workspace_root = fixture.workspace_root(&identity);
    let vault_dir = identity
        .synthetic_vault_root()
        .unwrap_or_else(|| panic!("Training must derive a Vault root"));
    fs::create_dir_all(&vault_dir).unwrap_or_else(|error| panic!("vault setup failed: {error}"));
    fs::write(vault_dir.join("notes.md"), b"synthetic content")
        .unwrap_or_else(|error| panic!("note setup failed: {error}"));
    let mut ledger = fixture.ledger(&workspace_root);
    let id = seed_evidence(
        &mut ledger,
        "notes.md",
        None,
        EvidenceVerification::Unverified,
    );
    let vault = DesktopVault::resolve(&identity);
    let mut ids = TestIds(0);

    // Someone else re-observes first: the record is now at version 2.
    let ReobserveResult::Persisted(advanced) = reobserve(
        &mut ledger,
        &vault,
        &id,
        AggregateVersion::initial(),
        idempotency("synthetic-reobserve-1"),
        correlation("synthetic-reobserve-correlation-1"),
        &mut ids,
        UtcTimestamp::from_unix_millis(2_000),
    )
    .unwrap_or_else(|error| panic!("first reobserve failed: {error:?}")) else {
        panic!("the first observation of an unverified reference must persist");
    };
    let revision_before = ledger
        .revision()
        .unwrap_or_else(|error| panic!("revision read failed: {error:?}"));

    // A view still showing version 1 clicks "pin".
    let error = pin_fingerprint(
        &mut ledger,
        &vault,
        &id,
        AggregateVersion::initial(),
        idempotency("synthetic-pin-stale"),
        correlation("synthetic-pin-stale-correlation"),
        &mut ids,
        UtcTimestamp::from_unix_millis(3_000),
    )
    .expect_err("a stale view must not pin");
    let EvidenceWriteError::VersionConflict { current } = error else {
        panic!("expected VersionConflict, got {error:?}");
    };
    assert_eq!(current, advanced.record.version);
    assert_eq!(
        ledger
            .revision()
            .unwrap_or_else(|error| panic!("revision read failed: {error:?}")),
        revision_before
    );

    // With the version it actually holds, the same pin goes through.
    pin_fingerprint(
        &mut ledger,
        &vault,
        &id,
        advanced.record.version,
        idempotency("synthetic-pin-fresh"),
        correlation("synthetic-pin-fresh-correlation"),
        &mut ids,
        UtcTimestamp::from_unix_millis(4_000),
    )
    .unwrap_or_else(|error| panic!("pin with the current version failed: {error:?}"));
}

fn seed_product(writer: &mut SqliteProductLedger) -> ProductId {
    let id =
        ProductId::parse("synthetic-product-1").unwrap_or_else(|error| panic!("id: {error:?}"));
    writer
        .create_product(
            CreateProduct {
                id: id.clone(),
                name: ShortText::parse("Synthetic Product")
                    .unwrap_or_else(|error| panic!("name: {error:?}")),
                details: LongText::parse("A training record with no real counterpart.")
                    .unwrap_or_else(|error| panic!("details: {error:?}")),
                classification: Some(DataClassification::Internal),
                provenance: Provenance::UserEntered,
                context: PortfolioContext {
                    idempotency_id: idempotency("synthetic-product-create-1"),
                    correlation_id: correlation("synthetic-product-correlation-1"),
                },
            },
            AuditEventId::parse("synthetic-product-audit-1")
                .unwrap_or_else(|error| panic!("id: {error:?}")),
            UtcTimestamp::from_unix_millis(900),
        )
        .unwrap_or_else(|error| panic!("product seed failed: {error:?}"));
    id
}

fn domain_code(error: &EvidenceWriteError) -> ErrorCode {
    match error {
        EvidenceWriteError::Write(LedgerTransactionError::Operation(domain)) => domain.code(),
        other => panic!("expected a Ledger domain refusal, got {other:?}"),
    }
}

/// The reason link belongs in this slice while relocate does not: it is a
/// Ledger write between two identities the person read, so it works with no
/// Vault at all. Run in the Live workspace, which has none, to prove it.
#[test]
fn linking_needs_no_vault_and_works_where_no_vault_is_configured() {
    let fixture = Fixture::new("link-live");
    let identity = fixture.identity(WorkspaceKind::Live);
    let workspace_root = fixture.workspace_root(&identity);
    let mut ledger = fixture.ledger(&workspace_root);
    let product = seed_product(&mut ledger);
    let id = seed_evidence(
        &mut ledger,
        "notes.md",
        None,
        EvidenceVerification::Unverified,
    );
    assert_eq!(
        DesktopVault::resolve(&identity).root().unwrap_err(),
        VaultUnavailable::NotConfigured
    );
    let mut ids = TestIds(0);

    let outcome = link_to_product(
        &mut ledger,
        product.clone(),
        &id,
        AggregateVersion::initial(),
        idempotency("synthetic-link-1"),
        correlation("synthetic-link-correlation-1"),
        &mut ids,
        UtcTimestamp::from_unix_millis(2_000),
    )
    .unwrap_or_else(|error| panic!("link failed: {error:?}"));
    assert_eq!(outcome.record.evidence_id, id);
    assert_eq!(outcome.record.classification, DataClassification::Internal);

    // The same pair again is the Ledger's own conflict, not a host error.
    let again = link_to_product(
        &mut ledger,
        product,
        &id,
        AggregateVersion::initial(),
        idempotency("synthetic-link-2"),
        correlation("synthetic-link-correlation-2"),
        &mut ids,
        UtcTimestamp::from_unix_millis(3_000),
    )
    .expect_err("an already-linked pair must be refused");
    assert_eq!(domain_code(&again), ErrorCode::DomainConflict);
}

/// A Product the Ledger does not hold is a domain not-found, carried through
/// untouched -- the facade adds nothing and hides nothing.
#[test]
fn linking_to_an_unknown_product_is_the_ledgers_own_not_found() {
    let fixture = Fixture::new("link-unknown");
    let identity = fixture.identity(WorkspaceKind::Training);
    let workspace_root = fixture.workspace_root(&identity);
    let mut ledger = fixture.ledger(&workspace_root);
    let id = seed_evidence(
        &mut ledger,
        "notes.md",
        None,
        EvidenceVerification::Unverified,
    );
    let mut ids = TestIds(0);

    let error = link_to_product(
        &mut ledger,
        ProductId::parse("synthetic-product-missing")
            .unwrap_or_else(|error| panic!("id: {error:?}")),
        &id,
        AggregateVersion::initial(),
        idempotency("synthetic-link-1"),
        correlation("synthetic-link-correlation-1"),
        &mut ids,
        UtcTimestamp::from_unix_millis(2_000),
    )
    .expect_err("an unknown Product must be refused");
    assert_eq!(domain_code(&error), ErrorCode::DomainNotFound);
}

/// Proves the *order*, not only the refusal. The file is deleted before the
/// stale click: had the source been observed first, the pin flow would have
/// refused with `SourceNotObservable` (nothing to hash). It must refuse with
/// `VersionConflict`, which it can only do by comparing before observing.
#[test]
fn a_stale_pin_is_refused_before_the_source_is_observed() {
    let fixture = Fixture::new("stale-pin-order");
    let identity = fixture.identity(WorkspaceKind::Training);
    let workspace_root = fixture.workspace_root(&identity);
    let vault_dir = identity
        .synthetic_vault_root()
        .unwrap_or_else(|| panic!("Training must derive a Vault root"));
    fs::create_dir_all(&vault_dir).unwrap_or_else(|error| panic!("vault setup failed: {error}"));
    let note = vault_dir.join("notes.md");
    fs::write(&note, b"synthetic content")
        .unwrap_or_else(|error| panic!("note setup failed: {error}"));
    let mut ledger = fixture.ledger(&workspace_root);
    let id = seed_evidence(
        &mut ledger,
        "notes.md",
        None,
        EvidenceVerification::Unverified,
    );
    let vault = DesktopVault::resolve(&identity);
    let mut ids = TestIds(0);

    // Someone else re-observes: version 2.
    let ReobserveResult::Persisted(advanced) = reobserve(
        &mut ledger,
        &vault,
        &id,
        AggregateVersion::initial(),
        idempotency("synthetic-reobserve-1"),
        correlation("synthetic-reobserve-correlation-1"),
        &mut ids,
        UtcTimestamp::from_unix_millis(2_000),
    )
    .unwrap_or_else(|error| panic!("first reobserve failed: {error:?}")) else {
        panic!("the first observation of an unverified reference must persist");
    };

    // The file is gone. An observation now would say SourceNotObservable.
    fs::remove_file(&note).unwrap_or_else(|error| panic!("note removal failed: {error}"));

    let error = pin_fingerprint(
        &mut ledger,
        &vault,
        &id,
        AggregateVersion::initial(),
        idempotency("synthetic-pin-stale"),
        correlation("synthetic-pin-stale-correlation"),
        &mut ids,
        UtcTimestamp::from_unix_millis(3_000),
    )
    .expect_err("a stale view must not pin");
    let EvidenceWriteError::VersionConflict { current } = error else {
        panic!("expected VersionConflict before any observation, got {error:?}");
    };
    assert_eq!(current, advanced.record.version);
}

/// The same rule for re-observe: a stale version is refused, and nothing is
/// written -- the Ledger revision does not move.
#[test]
fn a_stale_reobserve_is_refused_and_writes_nothing() {
    let fixture = Fixture::new("stale-reobserve");
    let identity = fixture.identity(WorkspaceKind::Training);
    let workspace_root = fixture.workspace_root(&identity);
    let vault_dir = identity
        .synthetic_vault_root()
        .unwrap_or_else(|| panic!("Training must derive a Vault root"));
    fs::create_dir_all(&vault_dir).unwrap_or_else(|error| panic!("vault setup failed: {error}"));
    fs::write(vault_dir.join("notes.md"), b"synthetic content")
        .unwrap_or_else(|error| panic!("note setup failed: {error}"));
    let mut ledger = fixture.ledger(&workspace_root);
    let id = seed_evidence(
        &mut ledger,
        "notes.md",
        None,
        EvidenceVerification::Unverified,
    );
    let vault = DesktopVault::resolve(&identity);
    let mut ids = TestIds(0);

    let ReobserveResult::Persisted(advanced) = reobserve(
        &mut ledger,
        &vault,
        &id,
        AggregateVersion::initial(),
        idempotency("synthetic-reobserve-1"),
        correlation("synthetic-reobserve-correlation-1"),
        &mut ids,
        UtcTimestamp::from_unix_millis(2_000),
    )
    .unwrap_or_else(|error| panic!("first reobserve failed: {error:?}")) else {
        panic!("the first observation of an unverified reference must persist");
    };
    let revision_before = ledger
        .revision()
        .unwrap_or_else(|error| panic!("revision read failed: {error:?}"));

    let error = reobserve(
        &mut ledger,
        &vault,
        &id,
        AggregateVersion::initial(),
        idempotency("synthetic-reobserve-stale"),
        correlation("synthetic-reobserve-stale-correlation"),
        &mut ids,
        UtcTimestamp::from_unix_millis(3_000),
    )
    .expect_err("a stale view must not re-observe");
    let EvidenceWriteError::VersionConflict { current } = error else {
        panic!("expected VersionConflict, got {error:?}");
    };
    assert_eq!(current, advanced.record.version);
    assert_eq!(
        ledger
            .revision()
            .unwrap_or_else(|error| panic!("revision read failed: {error:?}")),
        revision_before
    );
}
