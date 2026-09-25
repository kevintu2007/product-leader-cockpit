//! The seed's own proof: it refuses Live, it produces what it says, every
//! read route composes something from it, and a re-run is the same content.
//!
//! Every test uses its own protected application directory under the OS
//! app-data base, so the real desktop directory is never touched.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_application::cockpit_adapter::products_from_snapshot;
use pmc_application::people_adapter::directory_entries_from_snapshot;
use pmc_application::product_adapter::product_detail_facts_from_snapshots;
use pmc_application::work_queue_adapter::work_item_facts_from_snapshots;
use pmc_domain::attention::AttentionThresholds;
use pmc_domain::classification::DataClassification;
use pmc_domain::composition_source::LedgerSnapshotForCompositionPort;
use pmc_domain::projection_source::LedgerSnapshotForProjectionPort;
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::EvidenceVerification;
use pmc_ledger::sqlite::{SqliteProductLedger, CURRENT_SCHEMA_VERSION};
use pmc_platform::settings::ProtectedSettingsRoot;
use pmc_platform::workspace::{WorkspaceIdentity, WorkspaceKind};
use pmc_seed::{
    read_manifest, seed_training_in, SeedError, SeedOptions, LEDGER_FILE_NAME, MANIFEST_FILE_NAME,
    SEED_CLOCK_MILLIS, SEED_ID, SUPPORTED_SCHEMA_VERSION,
};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A protected application directory that exists only for one test.
struct TestRoot(String);

/// This repo's Windows sandbox redirects writes under the real
/// `%LOCALAPPDATA%` into an app-package container, which trips
/// `ProtectedSettingsRoot::prepare`'s canonical-path defence for a benign
/// reason. Point the *test process's* `%LOCALAPPDATA%` at a plain directory
/// under `target/` once, exactly as `pmc-platform`'s own tests do. Production
/// is untouched. `std::env::set_var` is safe in this edition and no thread
/// has been spawned yet when `Once` runs it.
#[cfg(windows)]
fn ensure_test_app_data_root_is_not_redirected() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("pmc-seed-test-app-data");
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
        Self(format!("PmcSeedTest-{label}-{nonce}-{sequence}"))
    }

    fn name(&self) -> &str {
        &self.0
    }

    fn protected_root(&self) -> ProtectedSettingsRoot {
        ProtectedSettingsRoot::prepare(&self.0).unwrap()
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        if let Ok(root) = ProtectedSettingsRoot::prepare(&self.0) {
            let _ = fs::remove_dir_all(root.path());
        }
    }
}

fn seeded(label: &str) -> (TestRoot, pmc_seed::SeedReport) {
    let root = TestRoot::new(label);
    let report = seed_training_in(root.name(), SeedOptions::default()).unwrap();
    (root, report)
}

fn open(report: &pmc_seed::SeedReport) -> SqliteProductLedger {
    SqliteProductLedger::open(report.training_root.join(LEDGER_FILE_NAME)).unwrap()
}

#[test]
fn the_live_workspace_can_never_mint_a_seed_permit() {
    let root = TestRoot::new("live");
    let live = WorkspaceIdentity::resolve(&root.protected_root(), WorkspaceKind::Live).unwrap();
    assert!(live.synthetic_seed_policy().authorize().is_err());
}

/// Building the seed is the slow part of this suite: every command runs in
/// its own durable transaction. The proofs below only read what one seed
/// wrote, so they share one build instead of making their own. Each keeps
/// its own function, so a failure still names the proof that broke. The
/// proofs that change the workspace -- a re-run, a reset refused by a link,
/// a foreign workspace -- keep their own roots.
#[test]
fn one_seed_proves_what_it_wrote_and_where() {
    let (root, report) = seeded("shared");
    only_training_is_written(&root, &report);
    a_fresh_seed_opens_at_the_pinned_schema_with_the_manifest_counts(&report);
    the_snapshots_carry_what_the_manifest_counts_and_every_evidence_state(&report);
    every_read_route_composes_something_from_the_seed(&report);
    nothing_the_seed_writes_names_the_host(&root, &report);
}

/// The seed's only entry point never resolves anything but Training: after
/// a seed, the Live directory does not exist at all.
fn only_training_is_written(root: &TestRoot, report: &pmc_seed::SeedReport) {
    assert!(report.training_root.ends_with("training"));
    assert!(!root
        .protected_root()
        .path()
        .join("workspaces")
        .join("live")
        .exists());
}

fn a_fresh_seed_opens_at_the_pinned_schema_with_the_manifest_counts(report: &pmc_seed::SeedReport) {
    assert_eq!(SUPPORTED_SCHEMA_VERSION, CURRENT_SCHEMA_VERSION);
    let ledger = open(report);
    assert_eq!(ledger.schema_version(), SUPPORTED_SCHEMA_VERSION);
    assert_eq!(ledger.revision().unwrap(), report.manifest.ledger_revision);

    let counts = &report.manifest.counts;
    assert_eq!(counts["portfolios"], 1);
    assert_eq!(counts["products"], 4);
    assert_eq!(counts["initiatives"], 5);
    assert_eq!(counts["projects"], 8);
    assert_eq!(counts["milestones"], 12);
    assert_eq!(counts["stakeholders"], 8);
    assert_eq!(counts["stakeholder_relationships"], 13);
    assert_eq!(counts["action_requests"], 6);
    assert_eq!(counts["decision_requests"], 3);
    assert_eq!(counts["risks"], 4);
    assert_eq!(counts["issues"], 4);
    assert_eq!(counts["evidence_references"], 6);
    assert_eq!(counts["evidence_links"], 6);

    let stored = read_manifest(&report.training_root).unwrap().unwrap();
    assert_eq!(stored, report.manifest);
    assert_eq!(stored.seed_id, SEED_ID);
}

fn the_snapshots_carry_what_the_manifest_counts_and_every_evidence_state(
    report: &pmc_seed::SeedReport,
) {
    let ledger = open(report);
    let as_of = UtcTimestamp::from_unix_millis(SEED_CLOCK_MILLIS);
    let composition = ledger.read_composition_snapshot(as_of).unwrap();
    let projection = ledger.read_projection_snapshot(as_of).unwrap();

    assert_eq!(composition.products.len(), 4);
    assert_eq!(composition.stakeholders.len(), 8);
    assert_eq!(composition.stakeholder_relationships.len(), 13);
    assert_eq!(composition.action_requests.len(), 6);
    assert_eq!(composition.decision_requests.len(), 3);
    assert_eq!(composition.issues.len(), 4);
    assert_eq!(composition.milestones.len(), 12);
    assert_eq!(composition.evidence_references.len(), 6);
    assert_eq!(composition.evidence_links.len(), 6);
    assert_eq!(projection.products.len(), 4);
    assert_eq!(projection.projects.len(), 8);
    assert_eq!(projection.risks.len(), 4);
    assert_eq!(projection.kpis.len(), 8);

    let state = |id: &str| {
        composition
            .evidence_references
            .iter()
            .find(|reference| reference.id.as_str() == id)
            .map(|reference| (reference.verification.clone(), reference.pinned))
            .unwrap()
    };
    assert!(matches!(
        state("demo-evidence-1"),
        (EvidenceVerification::Verified { .. }, true)
    ));
    assert!(matches!(
        state("demo-evidence-2"),
        (EvidenceVerification::ObservedUnpinned { .. }, false)
    ));
    assert!(matches!(
        state("demo-evidence-3"),
        (EvidenceVerification::DegradedLastVerified { .. }, true)
    ));
    assert!(matches!(
        state("demo-evidence-4"),
        (EvidenceVerification::Unverified, false)
    ));
    assert_eq!(
        composition
            .evidence_references
            .iter()
            .find(|reference| reference.id.as_str() == "demo-evidence-5")
            .map(|reference| reference.classification),
        Some(DataClassification::Restricted)
    );
    assert!(composition
        .evidence_links
        .iter()
        .any(|link| link.target_type == "action_request"));
}

fn every_read_route_composes_something_from_the_seed(report: &pmc_seed::SeedReport) {
    let ledger = open(report);
    // The desktop reads at Date.now(); the seed's deadlines are fixed
    // calendar dates in the past, so any as_of after them sees overdue work.
    let as_of = UtcTimestamp::from_unix_millis(SEED_CLOCK_MILLIS + 30 * 86_400_000);
    let composition = ledger.read_composition_snapshot(as_of).unwrap();
    let projection = ledger.read_projection_snapshot(as_of).unwrap();
    let thresholds = AttentionThresholds::default();

    // S01 / S02 list
    assert_eq!(products_from_snapshot(&projection).len(), 4);
    // S03
    let work = work_item_facts_from_snapshots(&projection, &composition, thresholds);
    assert!(
        work.len() >= 13,
        "requests, decision requests, risks and issues: {}",
        work.len()
    );
    let with_attention = work
        .iter()
        .filter(|item| !item.attention.is_empty())
        .count();
    assert!(
        with_attention >= 3,
        "overdue and owner-less requests must raise attention: {with_attention}"
    );
    // S09
    let people = directory_entries_from_snapshot(&composition);
    assert_eq!(people.len(), 8);
    // O01 for each Product, including the Restricted fold on Delta
    for product in [
        "demo-product-1",
        "demo-product-2",
        "demo-product-3",
        "demo-product-4",
    ] {
        let facts =
            product_detail_facts_from_snapshots(product, &projection, &composition, thresholds)
                .unwrap();
        assert!(!facts.structure.is_empty(), "{product} must have structure");
        assert!(
            !facts.people.is_empty(),
            "{product} must have accountable people"
        );
    }
    let delta = product_detail_facts_from_snapshots(
        "demo-product-4",
        &projection,
        &composition,
        thresholds,
    )
    .unwrap();
    assert!(delta
        .evidence
        .iter()
        .any(|entry| entry.classification == DataClassification::Restricted));
}

#[test]
fn a_second_run_is_refused_without_reset_and_equal_with_it() {
    let (root, first) = seeded("rerun");
    let refused = seed_training_in(root.name(), SeedOptions::default());
    assert!(
        matches!(refused, Err(SeedError::AlreadySeeded)),
        "{refused:?}"
    );
    let again = seed_training_in(
        root.name(),
        SeedOptions {
            reset_training: true,
        },
    )
    .unwrap();
    assert_eq!(again.manifest, first.manifest);
    assert_eq!(
        again.manifest.logical_snapshot_sha256,
        first.manifest.logical_snapshot_sha256
    );
}

#[test]
fn a_workspace_that_holds_something_else_is_never_deleted() {
    let root = TestRoot::new("foreign");
    let training =
        WorkspaceIdentity::resolve(&root.protected_root(), WorkspaceKind::Training).unwrap();
    let path = training.root().as_path().to_path_buf();
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("not-ours.txt"), b"something a person put here").unwrap();

    let refused = seed_training_in(
        root.name(),
        SeedOptions {
            reset_training: true,
        },
    );

    assert!(
        matches!(refused, Err(SeedError::ForeignContents(_))),
        "{refused:?}"
    );
    assert!(path.join("not-ours.txt").exists());
    assert!(!path.join(MANIFEST_FILE_NAME).exists());
}

#[test]
fn a_link_inside_the_workspace_stops_the_reset_before_anything_is_deleted() {
    let (root, report) = seeded("link");
    let planted = report.training_root.join("planted-dir");
    let target = root.protected_root().path().join("outside-target");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("keep.txt"), b"must survive").unwrap();
    if create_directory_link(&target, &planted).is_err() {
        // No link privilege on this machine: nothing to assert.
        return;
    }

    let refused = seed_training_in(
        root.name(),
        SeedOptions {
            reset_training: true,
        },
    );

    assert!(
        matches!(refused, Err(SeedError::LinkInWorkspace(_))),
        "{refused:?}"
    );
    assert!(
        target.join("keep.txt").exists(),
        "the link's target must be untouched"
    );
    assert!(
        report.training_root.join(LEDGER_FILE_NAME).exists(),
        "nothing else may be deleted either"
    );
    let _ = fs::remove_dir(&planted);
}

fn nothing_the_seed_writes_names_the_host(root: &TestRoot, report: &pmc_seed::SeedReport) {
    let manifest = fs::read_to_string(report.training_root.join(MANIFEST_FILE_NAME)).unwrap();
    let host_root = root.protected_root().path().display().to_string();
    assert!(!manifest.contains(&host_root));
    assert!(!manifest.contains("C:\\"));
    assert!(!manifest.contains("/Users/"));
    let ledger = open(report);
    let composition = ledger
        .read_composition_snapshot(UtcTimestamp::from_unix_millis(SEED_CLOCK_MILLIS))
        .unwrap();
    let rendered = format!("{composition:?}");
    assert!(!rendered.contains(&host_root));
    assert!(
        !rendered.contains("demo-vault"),
        "no Vault path may reach the read surface"
    );
}

#[cfg(windows)]
fn create_directory_link(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

#[cfg(unix)]
fn create_directory_link(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(any(windows, unix)))]
fn create_directory_link(_: &std::path::Path, _: &std::path::Path) -> std::io::Result<()> {
    Err(std::io::Error::other("no link support"))
}
