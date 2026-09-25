//! The sample workspace's synthetic data (item ⑨; the accepted sample-workspace
//! amendment §7): the **Training** workspace made visible. Moved here from
//! `tools/pmc-seed`, which stays as a thin development command, so the desktop
//! host can prepare the sample without spawning a process.
//!
//! It is filled through the same typed Ledger commands an application caller
//! uses -- never raw SQL -- so every record carries a real audit event,
//! idempotency claim, version and revision.
//!
//! Three properties are not negotiable:
//!
//! 1. **Live can never be seeded.** The target is hard-coded: the protected
//!    app root, the `Training` workspace, and the typed
//!    [`SyntheticSeedPermit`] that `WorkspaceKind::Live` refuses to mint. No
//!    argument takes a path or a workspace kind.
//! 2. **Nothing is approved on the user's behalf.** Every H2a transition
//!    records a Head-of-Products approval; a headless seed cannot honestly
//!    produce one, so Action Requests and Decision Requests are left `Open`
//!    and Risks and Issues are created in their initial state. What needs
//!    an approval is left for the user to approve in the app.
//! 3. **The seed fails closed on schema drift.** Every schema bump makes an
//!    older Ledger file unopenable by design (the Ledger design), so the seed pins
//!    the schema version it was written against and refuses to run against
//!    any other, and it re-creates the whole workspace rather than migrate.
//!
//! Everything written is synthetic and public-safe: invented identifiers and
//! names, `synthetic_fixture` provenance, and no host path in any record.

#![allow(clippy::result_large_err)]

use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::knowledge::{reobserve_evidence_verification, ReobserveEvidenceError};
use pmc_domain::actions::{
    ActionDetails, ActionOperationContext, ActionTitle, CreateActionRequestDraft,
    SubmitActionRequest,
};
use pmc_domain::classification::DataClassification;
use pmc_domain::composition_source::LedgerSnapshotForCompositionPort;
use pmc_domain::decisions::{
    CreateDecisionRequestDraft, DecisionOperationContext, DecisionSubject, DecisionText,
    SubmitDecisionRequest,
};
use pmc_domain::delivery::{
    CreateInitiative, CreateMilestone, CreateProject, DefinedOutcome,
    OperationContext as DeliveryContext, RecordName, VerificationCriteria,
};
use pmc_domain::error::DomainError;
use pmc_domain::evidence::{
    CreateEvidenceReference, EvidenceFingerprint, EvidenceLinkTarget, FingerprintAlgorithm,
    LinkEvidence, OperationContext as EvidenceContext, VaultRelativePath,
};
use pmc_domain::identity::{
    ActionRequestId, AggregateVersion, AuditEventId, CorrelationId, DecisionRequestId,
    EvidenceReferenceId, IdempotencyId, InitiativeId, IssueId, KpiId, KpiObservationId,
    MilestoneId, PortfolioId, ProductId, ProjectId, RelationshipId, RiskId, RoadmapId,
    StakeholderId,
};
use pmc_domain::issues::{CreateIssue, IssueDetails, IssueOperationContext, IssueTitle};
use pmc_domain::portfolio::{
    CreateKpiDefinition, CreateKpiObservation, CreatePortfolio, CreateProduct, CreateRoadmap,
    LongText, OperationContext as PortfolioContext, ShortText,
};
use pmc_domain::provenance::{Provenance, ProvenanceReference};
use pmc_domain::relationships::{
    CreateStakeholder, LinkInitiativeProject, LinkPortfolioInitiative, LinkPortfolioProduct,
    LinkProductKpi, LinkProductRoadmap, LinkProjectProduct, LinkStakeholderRelationship,
    OperationContext as RelationshipContext, StakeholderKind, StakeholderName,
    StakeholderRelationshipPurpose, StakeholderSubject,
};
use pmc_domain::risks::{CreateRisk, RiskDetails, RiskOperationContext, RiskTitle};
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::{EvidenceVerification, IntegrityDigest};
use pmc_knowledge::vault::VaultRoot;
use pmc_ledger::sqlite::{
    inspect_ledger, LedgerCompatibility, LedgerTransactionError, SqliteProductLedger,
    CURRENT_SCHEMA_VERSION,
};
use pmc_platform::filesystem::compute_sha256_fingerprint;
use pmc_platform::settings::{ProtectedSettingsRoot, SettingsError};
use pmc_platform::workspace::{
    SyntheticSeedPermit, WorkspaceError, WorkspaceIdentity, WorkspaceKind,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The desktop's protected settings directory. Duplicated from
/// `apps/desktop/src-tauri/src/ledger_state.rs` on purpose: the seed must
/// not depend on the Tauri host crate, and the two constants are asserted
/// equal by a test there.
pub const APPLICATION_DIRECTORY: &str = "ProductMissionControlDesktop";
/// The Ledger file name the desktop opens inside a workspace root.
pub const LEDGER_FILE_NAME: &str = "product-ledger.sqlite3";
// The demo Vault directory used to be a constant here. It moved to
// `pmc_platform::workspace`, which now derives it through
// `WorkspaceIdentity::synthetic_vault_root`, because the desktop host has to
// find the same directory in order to read Evidence out of it and two
// literals in two crates would drift the first time either moved.
/// The manifest the seed leaves beside the Ledger so a later run can tell
/// its own output from anything else.
pub const MANIFEST_FILE_NAME: &str = "pmc-seed-manifest.json";
/// Identity of this scenario. Bump `SEED_VERSION` when the scenario changes.
pub const SEED_ID: &str = "pmc-training-demo-v1";
pub const SEED_VERSION: u32 = 1;
/// The schema this scenario was written against. The seed refuses any other:
/// a silent re-seed against a moved schema would be a fixture nobody
/// reviewed.
pub const SUPPORTED_SCHEMA_VERSION: u32 = 48;

/// 2026-09-01T00:00:00Z. Every record is created at this instant, so two
/// runs produce the same logical content. Deadlines are fixed calendar dates
/// (below) rather than offsets from the run, because the desktop reads every
/// route at `Date.now()`: a deadline that is past on the calendar is overdue
/// in the app, whatever day the seed ran.
pub const SEED_CLOCK_MILLIS: i64 = 1_788_220_800_000;
const PROJECT_START_MILLIS: i64 = 1_780_272_000_000; // 2026-06-01
const OVERDUE_RESPONSE_MILLIS: i64 = 1_787_184_000_000; // 2026-08-20
const PAST_MILESTONE_MILLIS: i64 = 1_786_752_000_000; // 2026-08-15
const FUTURE_MILESTONE_MILLIS: i64 = 1_795_996_800_000; // 2026-11-30
const PROJECT_END_MILLIS: i64 = 1_798_675_200_000; // 2026-12-31
const FUTURE_DUE_MILLIS: i64 = 1_803_859_200_000; // 2027-03-01

/// What the caller asked for.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SeedOptions {
    /// Delete the existing Training workspace first. Without it, a workspace
    /// that already holds this seed is refused rather than silently rebuilt.
    pub reset_training: bool,
}

/// What the seed wrote, for the console and for tests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SeedReport {
    pub training_root: PathBuf,
    pub manifest: SeedManifest,
}

/// Recorded beside the Ledger. Paths are workspace-relative only.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SeedManifest {
    pub seed_id: String,
    pub seed_version: u32,
    pub schema_version: u32,
    pub seed_clock_millis: i64,
    pub generator_version: String,
    pub ledger_revision: u64,
    pub counts: BTreeMap<String, usize>,
    pub vault_files: Vec<VaultFileEntry>,
    /// SHA-256 over the composition snapshot read at the seed clock; equal
    /// across runs when the logical content is equal. Raw SQLite bytes are
    /// deliberately not compared: WAL state and page layout are not content.
    pub logical_snapshot_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VaultFileEntry {
    pub relative_path: String,
    pub sha256: String,
}

/// Every way the seed refuses. Each variant means nothing further was done.
#[derive(Debug)]
pub enum SeedError {
    Settings(SettingsError),
    Workspace(WorkspaceError),
    /// The seed was asked to run against a schema it was not written for.
    SchemaMismatch {
        supported: u32,
        current: u32,
    },
    /// The Training workspace already holds this seed; pass `reset_training`.
    AlreadySeeded,
    /// The Training workspace holds something that is not this seed's output.
    ForeignContents(String),
    /// A link or reparse point inside the Training workspace; nothing is deleted.
    LinkInWorkspace(PathBuf),
    Io(io::Error),
    Ledger(String),
    Vault(String),
    Manifest(String),
    /// A fixed scenario value failed a domain parser -- a bug in this crate.
    Vocabulary(&'static str),
}

impl Display for SeedError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Settings(error) => write!(formatter, "protected settings root: {error}"),
            Self::Workspace(error) => write!(formatter, "workspace: {error}"),
            Self::SchemaMismatch { supported, current } => write!(
                formatter,
                "this seed was written for schema {supported} but the Ledger is at schema {current}; update the scenario before seeding"
            ),
            Self::AlreadySeeded => write!(
                formatter,
                "the Training workspace already holds this seed; pass --reset-training to rebuild it"
            ),
            Self::ForeignContents(what) => write!(
                formatter,
                "the Training workspace holds something this seed did not write ({what}); refusing to delete it"
            ),
            Self::LinkInWorkspace(path) => write!(
                formatter,
                "a link or reparse point sits inside the Training workspace ({}); refusing to delete anything",
                path.display()
            ),
            Self::Io(error) => write!(formatter, "filesystem: {error}"),
            Self::Ledger(detail) => write!(formatter, "ledger command: {detail}"),
            Self::Vault(detail) => write!(formatter, "demo vault: {detail}"),
            Self::Manifest(detail) => write!(formatter, "manifest: {detail}"),
            Self::Vocabulary(what) => write!(formatter, "scenario vocabulary rejected: {what}"),
        }
    }
}

impl std::error::Error for SeedError {}

impl From<SettingsError> for SeedError {
    fn from(error: SettingsError) -> Self {
        Self::Settings(error)
    }
}
impl From<WorkspaceError> for SeedError {
    fn from(error: WorkspaceError) -> Self {
        Self::Workspace(error)
    }
}
impl From<io::Error> for SeedError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
impl From<LedgerTransactionError<DomainError>> for SeedError {
    fn from(error: LedgerTransactionError<DomainError>) -> Self {
        Self::Ledger(format!("{error:?}"))
    }
}
impl From<ReobserveEvidenceError> for SeedError {
    fn from(error: ReobserveEvidenceError) -> Self {
        Self::Vault(format!("{error:?}"))
    }
}

/// Seed the Training workspace under the desktop's protected root.
pub fn seed_training(options: SeedOptions) -> Result<SeedReport, SeedError> {
    seed_training_in(APPLICATION_DIRECTORY, options)
}

/// Seed the Training workspace under a named protected application
/// directory. Tests pass a unique name so they never touch the real one; the
/// binary passes [`APPLICATION_DIRECTORY`].
pub fn seed_training_in(
    application_directory: &str,
    options: SeedOptions,
) -> Result<SeedReport, SeedError> {
    if CURRENT_SCHEMA_VERSION != SUPPORTED_SCHEMA_VERSION {
        return Err(SeedError::SchemaMismatch {
            supported: SUPPORTED_SCHEMA_VERSION,
            current: CURRENT_SCHEMA_VERSION,
        });
    }
    let protected_root = ProtectedSettingsRoot::prepare(application_directory)?;
    let identity = WorkspaceIdentity::resolve(&protected_root, WorkspaceKind::Training)?;
    // The only way to a permit is through the policy of a resolved Training
    // identity; Live returns LiveSyntheticSeedDenied here.
    let permit = identity.synthetic_seed_policy().authorize()?;
    let root = identity.root().as_path().to_path_buf();

    prepare_root(&identity, permit, &root, options)?;
    write_seed(&identity, root)
}

/// What the sample folder holds now (the accepted sample-workspace
/// amendment §7).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SampleState {
    /// No folder, or an empty one.
    Absent,
    /// This seed's manifest, at this schema, listing exactly this seed's
    /// Vault files, each present and unchanged, and a Ledger at this schema.
    Current(SeedManifest),
    /// PMC's own sample, but incomplete, changed, or from an older seed or
    /// schema.
    Outdated(&'static str),
    /// Something PMC did not write; never deleted.
    Foreign(String),
}

/// The names PMC itself writes at the top of the sample folder: the Ledger
/// with the files SQLite keeps beside it, the manifest, and the synthetic
/// Vault. Inside the Vault anything goes — it is the sample's own Vault,
/// where a person learning PMC may add files (product owner, 2026-09-23) —
/// but a top-level entry PMC does not write means the folder is not only
/// PMC's.
fn is_own_top_level_name(name: &str, vault_name: &str) -> bool {
    name == LEDGER_FILE_NAME
        || name == MANIFEST_FILE_NAME
        || name == vault_name
        || ["-wal", "-shm", "-journal"]
            .iter()
            .any(|suffix| name == format!("{LEDGER_FILE_NAME}{suffix}"))
}

/// The Vault files this seed writes, re-derived from the scenario itself. A
/// manifest is compared against this whole list, never trusted on its own.
fn expected_vault_files() -> Vec<VaultFileEntry> {
    DEMO_VAULT_FILES
        .iter()
        .map(|(relative, body)| VaultFileEntry {
            relative_path: (*relative).to_owned(),
            sha256: hex(Sha256::digest(body.as_bytes())),
        })
        .collect()
}

/// Read what the sample folder holds, touching nothing (§7).
pub fn inspect_sample(identity: &WorkspaceIdentity) -> Result<SampleState, SeedError> {
    if identity.kind() != WorkspaceKind::Training {
        return Err(SeedError::Workspace(
            WorkspaceError::LiveSyntheticSeedDenied,
        ));
    }
    // Derived, never a second literal: `pmc_platform::workspace` owns it.
    let vault_dir = identity.synthetic_vault_root().ok_or(SeedError::Workspace(
        WorkspaceError::LiveSyntheticSeedDenied,
    ))?;
    inspect_folder(identity.root().as_path(), &vault_dir)
}

/// [`inspect_sample`] for a folder at `root` whose synthetic Vault is
/// `vault_dir` — the sample itself, or one built beside it. A link or
/// reparse point anywhere inside, or a top-level entry PMC does not write,
/// is foreign; a folder of PMC's own names without a manifest is an
/// interrupted build, so outdated.
pub(crate) fn inspect_folder(root: &Path, vault_dir: &Path) -> Result<SampleState, SeedError> {
    match fs::symlink_metadata(root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(SampleState::Absent),
        Err(error) => return Err(SeedError::Io(error)),
        Ok(metadata) => {
            if metadata.file_type().is_symlink()
                || is_reparse_point(&metadata)
                || !metadata.is_dir()
            {
                return Ok(SampleState::Foreign(
                    "the sample folder is a link".to_owned(),
                ));
            }
        }
    }
    // Every entry, all the way down: a link anywhere is refused.
    let mut files = Vec::new();
    let mut directories = Vec::new();
    match collect_for_reset(root, &mut files, &mut directories) {
        Ok(()) => {}
        Err(SeedError::LinkInWorkspace(_)) => {
            return Ok(SampleState::Foreign(
                "a link or reparse point inside the sample folder".to_owned(),
            ))
        }
        Err(error) => return Err(error),
    }
    let mut top_level = Vec::new();
    for entry in fs::read_dir(root)? {
        top_level.push(entry?.file_name().to_string_lossy().into_owned());
    }
    if top_level.is_empty() {
        return Ok(SampleState::Absent);
    }
    let vault_name = vault_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if let Some(unknown) = top_level
        .iter()
        .find(|name| !is_own_top_level_name(name, &vault_name))
    {
        return Ok(SampleState::Foreign(format!(
            "an entry PMC does not write: {unknown}"
        )));
    }
    let manifest = match read_manifest(root) {
        Ok(Some(manifest)) => manifest,
        Ok(None) => return Ok(SampleState::Outdated("no manifest: an interrupted build")),
        Err(SeedError::Manifest(_)) => {
            return Ok(SampleState::Foreign(
                "a manifest PMC cannot read".to_owned(),
            ))
        }
        Err(error) => return Err(error),
    };
    if manifest.seed_id != SEED_ID {
        return Ok(SampleState::Foreign(format!(
            "another seed: {}",
            manifest.seed_id
        )));
    }
    if manifest.seed_version != SEED_VERSION || manifest.schema_version != CURRENT_SCHEMA_VERSION {
        return Ok(SampleState::Outdated("an older seed or schema"));
    }
    if manifest.vault_files != expected_vault_files() {
        return Ok(SampleState::Outdated(
            "the manifest does not list this seed's Vault files",
        ));
    }
    if !vault_dir.is_dir() {
        return Ok(SampleState::Outdated("the sample Vault is missing"));
    }
    for listed in &manifest.vault_files {
        let unchanged = fs::read(vault_dir.join(&listed.relative_path))
            .map(|bytes| hex(Sha256::digest(&bytes)) == listed.sha256)
            .unwrap_or(false);
        if !unchanged {
            return Ok(SampleState::Outdated(
                "a sample Vault file is missing or changed",
            ));
        }
    }
    // The Ledger itself, read-only: at this schema and valid. A write-ahead
    // log left by a session that did not close is recovered when it opens.
    match inspect_ledger(root.join(LEDGER_FILE_NAME)) {
        Ok(LedgerCompatibility::Current(_) | LedgerCompatibility::UncleanShutdown) => {}
        Ok(_) => {
            return Ok(SampleState::Outdated(
                "the Ledger is missing or not at this schema",
            ))
        }
        Err(_) => return Ok(SampleState::Outdated("the Ledger cannot be read")),
    }
    Ok(SampleState::Current(manifest))
}

/// Write the demo Vault into `vault_dir`, run the scenario into a Ledger in
/// `root` and leave the manifest — last, so a folder without one is known to
/// be an interrupted build. The Ledger is closed when this returns.
pub(crate) fn write_seed_at(root: &Path, vault_dir: &Path) -> Result<SeedReport, SeedError> {
    fs::create_dir_all(vault_dir)?;
    let vault_files = write_demo_vault(vault_dir)?;
    let vault =
        VaultRoot::validate(vault_dir).map_err(|error| SeedError::Vault(format!("{error:?}")))?;

    let ledger_path = root.join(LEDGER_FILE_NAME);
    let mut ledger = SqliteProductLedger::open(&ledger_path)
        .map_err(|error| SeedError::Ledger(format!("open: {error:?}")))?;
    let counts = Scenario::new().run(&mut ledger, &vault, vault_dir)?;

    let snapshot = ledger
        .read_composition_snapshot(UtcTimestamp::from_unix_millis(SEED_CLOCK_MILLIS))
        .map_err(|error| SeedError::Ledger(format!("snapshot: {error:?}")))?;
    drop(ledger);
    let manifest = SeedManifest {
        seed_id: SEED_ID.to_owned(),
        seed_version: SEED_VERSION,
        schema_version: CURRENT_SCHEMA_VERSION,
        seed_clock_millis: SEED_CLOCK_MILLIS,
        generator_version: env!("CARGO_PKG_VERSION").to_owned(),
        ledger_revision: snapshot.ledger_revision,
        counts,
        vault_files,
        logical_snapshot_sha256: hex(Sha256::digest(format!("{snapshot:?}").as_bytes())),
    };
    let serialized = serde_json::to_string_pretty(&manifest)
        .map_err(|error| SeedError::Manifest(error.to_string()))?;
    fs::write(root.join(MANIFEST_FILE_NAME), serialized)?;
    Ok(SeedReport {
        training_root: root.to_path_buf(),
        manifest,
    })
}

/// The development command's seed: into the Training folder itself.
fn write_seed(identity: &WorkspaceIdentity, root: PathBuf) -> Result<SeedReport, SeedError> {
    let vault_dir = identity.synthetic_vault_root().ok_or(SeedError::Workspace(
        WorkspaceError::LiveSyntheticSeedDenied,
    ))?;
    write_seed_at(&root, &vault_dir)
}

/// SHA-256 over every file under `root` — its path relative to `root`, its
/// length and the digest of its content, sorted — so a delete preview can be
/// derived again and compared whole. A link anywhere inside is refused.
pub(crate) fn inventory_sha256(root: &Path) -> Result<String, SeedError> {
    let mut files = Vec::new();
    let mut directories = Vec::new();
    collect_for_reset(root, &mut files, &mut directories)?;
    let relative = |path: &Path| -> Result<String, SeedError> {
        Ok(path
            .strip_prefix(root)
            .map_err(|_| SeedError::LinkInWorkspace(path.to_path_buf()))?
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/"))
    };
    let mut lines = Vec::with_capacity(files.len() + directories.len());
    // Directories too: an empty folder added or removed is a change.
    for directory in &directories {
        lines.push(format!("D\t{}", relative(directory)?));
    }
    for file in &files {
        let bytes = fs::read(file)?;
        lines.push(format!(
            "F\t{}\t{}\t{}",
            relative(file)?,
            bytes.len(),
            hex(Sha256::digest(&bytes))
        ));
    }
    lines.sort();
    Ok(hex(Sha256::digest(lines.join("\n").as_bytes())))
}

/// Remove a folder PMC wrote and the tree under it, never following a link:
/// a link anywhere inside stops the removal before anything is deleted.
pub(crate) fn remove_tree(path: &Path) -> Result<(), SeedError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(SeedError::Io(error)),
        Ok(metadata) => {
            if metadata.file_type().is_symlink()
                || is_reparse_point(&metadata)
                || !metadata.is_dir()
            {
                return Err(SeedError::LinkInWorkspace(path.to_path_buf()));
            }
        }
    }
    reset_root(path)?;
    fs::remove_dir(path)?;
    Ok(())
}

/// Read the manifest a previous run left, if any.
pub fn read_manifest(root: &Path) -> Result<Option<SeedManifest>, SeedError> {
    let path = root.join(MANIFEST_FILE_NAME);
    if fs::symlink_metadata(&path).is_err() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path)?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|error| SeedError::Manifest(error.to_string()))
}

// ---------------------------------------------------------------------------
// Workspace preparation: permit, foreign-content refusal, safe reset.
// ---------------------------------------------------------------------------

fn prepare_root(
    identity: &WorkspaceIdentity,
    permit: SyntheticSeedPermit,
    root: &Path,
    options: SeedOptions,
) -> Result<(), SeedError> {
    // Consumed before any filesystem effect; a mismatched permit stops here.
    identity.accept_synthetic_seed_permit(permit)?;
    match fs::symlink_metadata(root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(root)?;
            return Ok(());
        }
        Err(error) => return Err(SeedError::Io(error)),
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(SeedError::LinkInWorkspace(root.to_path_buf()));
            }
        }
    }
    let mut entries = fs::read_dir(root)?;
    if entries.next().is_none() {
        return Ok(());
    }
    match read_manifest(root)? {
        Some(manifest) if manifest.seed_id == SEED_ID => {}
        Some(manifest) => return Err(SeedError::ForeignContents(manifest.seed_id)),
        None => {
            return Err(SeedError::ForeignContents(
                "no seed manifest beside the contents".to_owned(),
            ))
        }
    }
    if !options.reset_training {
        return Err(SeedError::AlreadySeeded);
    }
    reset_root(root)
}

/// Delete the Training root's contents, never following a link. Two passes:
/// the first refuses if any entry is a link or reparse point, so a refused
/// tree is left entirely intact; the second deletes files then directories
/// bottom-up.
fn reset_root(root: &Path) -> Result<(), SeedError> {
    let mut files = Vec::new();
    let mut directories = Vec::new();
    collect_for_reset(root, &mut files, &mut directories)?;
    for file in files {
        fs::remove_file(&file)?;
    }
    for directory in directories.into_iter().rev() {
        fs::remove_dir(&directory)?;
    }
    Ok(())
}

fn collect_for_reset(
    directory: &Path,
    files: &mut Vec<PathBuf>,
    directories: &mut Vec<PathBuf>,
) -> Result<(), SeedError> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
            return Err(SeedError::LinkInWorkspace(path));
        }
        if metadata.is_dir() {
            directories.push(path.clone());
            collect_for_reset(&path, files, directories)?;
        } else if metadata.is_file() {
            files.push(path);
        } else {
            return Err(SeedError::LinkInWorkspace(path));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_: &fs::Metadata) -> bool {
    false
}

// ---------------------------------------------------------------------------
// Demo Vault: real files for the Evidence states the app can show.
// ---------------------------------------------------------------------------

const VAULT_PINNED: &str = "Research/market-scan.md";
const VAULT_UNPINNED: &str = "Research/customer-interviews.md";
const VAULT_DEGRADED: &str = "Research/vendor-quote.md";
const VAULT_RESTRICTED: &str = "Restricted/board-brief.md";
const VAULT_REQUEST: &str = "Requests/rollout-checklist.md";

/// The demo Vault's kept files and their exact contents.
const DEMO_VAULT_FILES: [(&str, &str); 4] = [
        (VAULT_PINNED, "# Demo market scan\n\nSynthetic content for the Training workspace. Public-safe.\n"),
        (VAULT_UNPINNED, "# Demo customer interviews\n\nSynthetic notes. No real customer appears here.\n"),
        (VAULT_RESTRICTED, "# Demo board brief\n\nSynthetic. Labelled Restricted only to exercise classification folding.\n"),
        (VAULT_REQUEST, "# Demo rollout checklist\n\n- [ ] synthetic step one\n- [ ] synthetic step two\n"),
];

fn write_demo_vault(vault_dir: &Path) -> Result<Vec<VaultFileEntry>, SeedError> {
    let mut entries = Vec::new();
    for (relative, body) in DEMO_VAULT_FILES {
        let path = vault_dir.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, body)?;
        entries.push(VaultFileEntry {
            relative_path: relative.to_owned(),
            sha256: hex(Sha256::digest(body.as_bytes())),
        });
    }
    // The degraded case: a file that existed when its reference was pinned
    // and verified, and is gone when it is next observed. Written here and
    // removed by the scenario after the pinned observation.
    let degraded = vault_dir.join(VAULT_DEGRADED);
    if let Some(parent) = degraded.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        &degraded,
        "# Demo vendor quote\n\nSynthetic. This file is removed after pinning.\n",
    )?;
    Ok(entries)
}

fn hex(digest: impl AsRef<[u8]>) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

// ---------------------------------------------------------------------------
// The scenario, through typed commands only.
// ---------------------------------------------------------------------------

struct Scenario {
    next_audit: u64,
    now: UtcTimestamp,
    provenance: Provenance,
}

impl Scenario {
    fn new() -> Self {
        Self {
            next_audit: 0,
            now: UtcTimestamp::from_unix_millis(SEED_CLOCK_MILLIS),
            provenance: Provenance::SyntheticFixture(
                ProvenanceReference::parse(SEED_ID).unwrap_or_else(|_| unreachable!()),
            ),
        }
    }

    fn audit(&mut self) -> Result<AuditEventId, SeedError> {
        self.next_audit += 1;
        AuditEventId::parse(format!("demo-audit-{}", self.next_audit))
            .map_err(|_| SeedError::Vocabulary("audit id"))
    }

    fn run(
        &mut self,
        ledger: &mut SqliteProductLedger,
        vault: &VaultRoot,
        vault_dir: &Path,
    ) -> Result<BTreeMap<String, usize>, SeedError> {
        let mut counts = BTreeMap::new();
        let provenance = self.provenance.clone();
        let internal = Some(DataClassification::Internal);

        // ---- Portfolio, Products, Roadmaps, KPIs -------------------------
        let portfolio = PortfolioId::parse("demo-portfolio-1")
            .map_err(|_| SeedError::Vocabulary("portfolio id"))?;
        ledger.create_portfolio(
            CreatePortfolio {
                id: portfolio.clone(),
                name: short("Demo Portfolio")?,
                details: long(
                    "Synthetic training portfolio. Every record here is invented and public-safe.",
                )?,
                classification: Some(DataClassification::Public),
                provenance: provenance.clone(),
                context: portfolio_context("portfolio-1")?,
            },
            self.audit()?,
            self.now,
        )?;
        counts.insert("portfolios".to_owned(), 1);

        let product_names = ["Atlas", "Beacon", "Cinder", "Delta"];
        let product_classes = [
            DataClassification::Internal,
            DataClassification::Internal,
            DataClassification::Confidential,
            DataClassification::Internal,
        ];
        let mut products = Vec::new();
        for (index, (name, class)) in product_names.iter().zip(product_classes).enumerate() {
            let number = index + 1;
            let id = ProductId::parse(format!("demo-product-{number}"))
                .map_err(|_| SeedError::Vocabulary("product id"))?;
            ledger.create_product(
                CreateProduct {
                    id: id.clone(),
                    name: short(&format!("Demo Product {name}"))?,
                    details: long(&format!(
                        "Synthetic product {name}: a training record with no real counterpart."
                    ))?,
                    classification: Some(class),
                    provenance: provenance.clone(),
                    context: portfolio_context(&format!("product-{number}"))?,
                },
                self.audit()?,
                self.now,
            )?;
            ledger.link_portfolio_product(
                LinkPortfolioProduct {
                    id: relationship(&format!("demo-rel-portfolio-product-{number}"))?,
                    portfolio_id: portfolio.clone(),
                    product_id: id.clone(),
                    expected_portfolio_version: AggregateVersion::initial(),
                    expected_product_version: AggregateVersion::initial(),
                    context: relationship_context(&format!("portfolio-product-{number}"))?,
                },
                self.audit()?,
                self.now,
            )?;
            products.push(id);
        }
        counts.insert("products".to_owned(), products.len());

        let mut roadmaps = 0;
        for (index, product) in products.iter().enumerate() {
            let number = index + 1;
            let id = RoadmapId::parse(format!("demo-roadmap-{number}"))
                .map_err(|_| SeedError::Vocabulary("roadmap id"))?;
            ledger.create_roadmap(
                CreateRoadmap {
                    id: id.clone(),
                    name: short(&format!("Demo Roadmap {number}"))?,
                    details: long("Synthetic roadmap for the Training workspace.")?,
                    classification: internal,
                    provenance: provenance.clone(),
                    context: portfolio_context(&format!("roadmap-{number}"))?,
                },
                self.audit()?,
                self.now,
            )?;
            ledger.link_product_roadmap(
                LinkProductRoadmap {
                    id: relationship(&format!("demo-rel-product-roadmap-{number}"))?,
                    product_id: product.clone(),
                    roadmap_id: id,
                    expected_product_version: AggregateVersion::initial(),
                    expected_roadmap_version: AggregateVersion::initial(),
                    context: relationship_context(&format!("product-roadmap-{number}"))?,
                },
                self.audit()?,
                self.now,
            )?;
            roadmaps += 1;
        }
        counts.insert("roadmaps".to_owned(), roadmaps);

        let kpi_names = [
            "Activation rate",
            "Weekly active accounts",
            "Support backlog",
            "Release lead time",
            "Renewal rate",
            "Defect escape rate",
            "Onboarding time",
            "Net revenue retention",
        ];
        let mut kpis = 0;
        let mut observations = 0;
        for (index, kpi_name) in kpi_names.iter().enumerate() {
            let number = index + 1;
            let product = &products[index % products.len()];
            let kpi = KpiId::parse(format!("demo-kpi-{number}"))
                .map_err(|_| SeedError::Vocabulary("kpi id"))?;
            ledger.create_kpi_definition(
                CreateKpiDefinition {
                    id: kpi.clone(),
                    name: short(&format!("Demo KPI: {kpi_name}"))?,
                    definition: long(
                        "Synthetic KPI definition used only in the Training workspace.",
                    )?,
                    owner: short("Demo KPI owner")?,
                    target: short(&format!("{}", 60 + number * 3))?,
                    cadence: short("Weekly")?,
                    source: long("Synthetic source; no system of record behind it.")?,
                    classification: internal,
                    provenance: provenance.clone(),
                    context: portfolio_context(&format!("kpi-{number}"))?,
                },
                self.audit()?,
                self.now,
            )?;
            ledger.link_product_kpi(
                LinkProductKpi {
                    id: relationship(&format!("demo-rel-product-kpi-{number}"))?,
                    product_id: product.clone(),
                    kpi_id: kpi.clone(),
                    expected_product_version: AggregateVersion::initial(),
                    expected_kpi_version: AggregateVersion::initial(),
                    context: relationship_context(&format!("product-kpi-{number}"))?,
                },
                self.audit()?,
                self.now,
            )?;
            kpis += 1;
            for sample in 1..=2u64 {
                let observation =
                    KpiObservationId::parse(format!("demo-kpi-observation-{number}-{sample}"))
                        .map_err(|_| SeedError::Vocabulary("observation id"))?;
                ledger.create_kpi_observation(
                    CreateKpiObservation {
                        id: observation,
                        kpi_id: kpi.clone(),
                        value: short(&format!("{}", 50 + number * 2 + sample as usize * 4))?,
                        observed_at: UtcTimestamp::from_unix_millis(
                            SEED_CLOCK_MILLIS - (3 - sample as i64) * 7 * 86_400_000,
                        ),
                        source: long("Synthetic observation.")?,
                        classification: internal,
                        provenance: provenance.clone(),
                        context: portfolio_context(&format!("observation-{number}-{sample}"))?,
                    },
                    self.audit()?,
                    self.now,
                )?;
                observations += 1;
            }
        }
        counts.insert("kpi_definitions".to_owned(), kpis);
        counts.insert("kpi_observations".to_owned(), observations);

        // ---- Initiatives, Projects, Milestones ----------------------------
        let initiative_names = [
            "Self-serve onboarding",
            "Reliability programme",
            "Partner channel",
            "Cost-to-serve reduction",
            "Compliance readiness",
        ];
        let mut initiatives = Vec::new();
        for (index, name) in initiative_names.iter().enumerate() {
            let number = index + 1;
            let id = InitiativeId::parse(format!("demo-initiative-{number}"))
                .map_err(|_| SeedError::Vocabulary("initiative id"))?;
            ledger.create_initiative(
                CreateInitiative {
                    context: delivery_context(&format!("initiative-{number}"))?,
                    id: id.clone(),
                    name: record_name(&format!("Demo Initiative: {name}"))?,
                    defined_outcome: outcome(
                        "A synthetic, measurable outcome for the Training workspace.",
                    )?,
                    classification: internal,
                    provenance: provenance.clone(),
                },
                self.audit()?,
                self.now,
            )?;
            ledger.link_portfolio_initiative(
                LinkPortfolioInitiative {
                    id: relationship(&format!("demo-rel-portfolio-initiative-{number}"))?,
                    portfolio_id: portfolio.clone(),
                    initiative_id: id.clone(),
                    expected_portfolio_version: AggregateVersion::initial(),
                    expected_initiative_version: AggregateVersion::initial(),
                    context: relationship_context(&format!("portfolio-initiative-{number}"))?,
                },
                self.audit()?,
                self.now,
            )?;
            initiatives.push(id);
        }
        counts.insert("initiatives".to_owned(), initiatives.len());

        let mut projects = Vec::new();
        for number in 1..=8usize {
            let id = ProjectId::parse(format!("demo-project-{number}"))
                .map_err(|_| SeedError::Vocabulary("project id"))?;
            ledger.create_project(
                CreateProject {
                    context: delivery_context(&format!("project-{number}"))?,
                    id: id.clone(),
                    name: record_name(&format!("Demo Project {number}"))?,
                    start_at: UtcTimestamp::from_unix_millis(PROJECT_START_MILLIS),
                    end_at: UtcTimestamp::from_unix_millis(PROJECT_END_MILLIS),
                    classification: internal,
                    provenance: provenance.clone(),
                },
                self.audit()?,
                self.now,
            )?;
            let initiative = &initiatives[(number - 1) % initiatives.len()];
            ledger.link_initiative_project(
                LinkInitiativeProject {
                    id: relationship(&format!("demo-rel-initiative-project-{number}"))?,
                    initiative_id: initiative.clone(),
                    project_id: id.clone(),
                    expected_initiative_version: AggregateVersion::initial(),
                    expected_project_version: AggregateVersion::initial(),
                    context: relationship_context(&format!("initiative-project-{number}"))?,
                },
                self.audit()?,
                self.now,
            )?;
            let product = &products[(number - 1) % products.len()];
            ledger.link_project_product(
                LinkProjectProduct {
                    id: relationship(&format!("demo-rel-project-product-{number}"))?,
                    project_id: id.clone(),
                    product_id: product.clone(),
                    expected_project_version: AggregateVersion::initial(),
                    expected_product_version: AggregateVersion::initial(),
                    context: relationship_context(&format!("project-product-{number}"))?,
                },
                self.audit()?,
                self.now,
            )?;
            projects.push(id);
        }
        counts.insert("projects".to_owned(), projects.len());

        let mut milestones = Vec::new();
        for number in 1..=12usize {
            let project = &projects[(number - 1) % projects.len()];
            let id = MilestoneId::parse(format!("demo-milestone-{number}"))
                .map_err(|_| SeedError::Vocabulary("milestone id"))?;
            let due = if number % 3 == 0 {
                PAST_MILESTONE_MILLIS
            } else {
                FUTURE_MILESTONE_MILLIS
            };
            ledger.create_milestone(
                CreateMilestone {
                    context: delivery_context(&format!("milestone-{number}"))?,
                    id: id.clone(),
                    project_id: project.clone(),
                    name: record_name(&format!("Demo Milestone {number}"))?,
                    verification_criteria: criteria("Synthetic verification criteria.")?,
                    due_at: UtcTimestamp::from_unix_millis(due),
                    classification: None,
                    provenance: provenance.clone(),
                },
                self.audit()?,
                self.now,
            )?;
            milestones.push(id);
        }
        counts.insert("milestones".to_owned(), milestones.len());

        // ---- Stakeholders and relationships ------------------------------
        let people = [
            ("Demo Person: Head of Delivery", StakeholderKind::Person),
            ("Demo Person: Product Lead Atlas", StakeholderKind::Person),
            ("Demo Person: Product Lead Beacon", StakeholderKind::Person),
            ("Demo Person: Platform Architect", StakeholderKind::Person),
            (
                "Demo Person: Customer Success Lead",
                StakeholderKind::Person,
            ),
            ("Demo Person: Finance Partner", StakeholderKind::Person),
            ("Demo Org: Launch Partner", StakeholderKind::Organization),
            ("Demo Org: Hosting Vendor", StakeholderKind::Organization),
        ];
        let mut stakeholders = Vec::new();
        for (index, (name, kind)) in people.iter().enumerate() {
            let number = index + 1;
            let id = StakeholderId::parse(format!("demo-stakeholder-{number}"))
                .map_err(|_| SeedError::Vocabulary("stakeholder id"))?;
            ledger.create_stakeholder(
                CreateStakeholder {
                    id: id.clone(),
                    name: StakeholderName::parse(*name)
                        .map_err(|_| SeedError::Vocabulary("stakeholder name"))?,
                    kind: *kind,
                    classification: Some(if number == 8 {
                        DataClassification::Confidential
                    } else {
                        DataClassification::Internal
                    }),
                    provenance: provenance.clone(),
                    context: relationship_context(&format!("stakeholder-{number}"))?,
                },
                self.audit()?,
                self.now,
            )?;
            stakeholders.push(id);
        }
        counts.insert("stakeholders".to_owned(), stakeholders.len());

        let mut relationships: Vec<(
            StakeholderId,
            StakeholderSubject,
            StakeholderRelationshipPurpose,
        )> = Vec::new();
        for (index, product) in products.iter().enumerate() {
            relationships.push((
                stakeholders[(index + 1) % 6].clone(),
                StakeholderSubject::Product(product.clone()),
                StakeholderRelationshipPurpose::Responsibility,
            ));
        }
        for (index, project) in projects.iter().take(4).enumerate() {
            relationships.push((
                stakeholders[6 + index % 2].clone(),
                StakeholderSubject::Project(project.clone()),
                StakeholderRelationshipPurpose::Dependency,
            ));
        }
        relationships.push((
            stakeholders[0].clone(),
            StakeholderSubject::Initiative(initiatives[0].clone()),
            StakeholderRelationshipPurpose::Responsibility,
        ));
        relationships.push((
            stakeholders[3].clone(),
            StakeholderSubject::Initiative(initiatives[1].clone()),
            StakeholderRelationshipPurpose::Responsibility,
        ));
        relationships.push((
            stakeholders[5].clone(),
            StakeholderSubject::Portfolio(portfolio.clone()),
            StakeholderRelationshipPurpose::Dependency,
        ));
        // Not a Milestone subject: the SQLite writer refuses
        // `StakeholderSubject::Milestone` outright (relationship_repository).
        relationships.push((
            stakeholders[4].clone(),
            StakeholderSubject::Project(projects[4].clone()),
            StakeholderRelationshipPurpose::Responsibility,
        ));
        // The same person accountable for two Products, so O01 can show
        // "other Products accountable for".
        relationships.push((
            stakeholders[1].clone(),
            StakeholderSubject::Product(products[3].clone()),
            StakeholderRelationshipPurpose::Responsibility,
        ));
        for (index, (stakeholder, subject, purpose)) in relationships.iter().enumerate() {
            let number = index + 1;
            ledger.link_stakeholder_relationship(
                LinkStakeholderRelationship {
                    id: relationship(&format!("demo-rel-stakeholder-{number}"))?,
                    stakeholder_id: stakeholder.clone(),
                    subject: subject.clone(),
                    purpose: *purpose,
                    expected_stakeholder_version: AggregateVersion::initial(),
                    expected_subject_version: AggregateVersion::initial(),
                    context: relationship_context(&format!("stakeholder-relationship-{number}"))?,
                },
                self.audit()?,
                self.now,
            )?;
        }
        counts.insert("stakeholder_relationships".to_owned(), relationships.len());

        // ---- Work: Requests left Open for the user to act on -------------
        let requests: [(&str, Option<usize>, i64); 6] = [
            (
                "Confirm the Atlas rollout window",
                Some(1),
                OVERDUE_RESPONSE_MILLIS,
            ),
            (
                "Approve the Beacon pricing experiment",
                Some(2),
                OVERDUE_RESPONSE_MILLIS,
            ),
            ("Review the Cinder vendor quote", Some(3), FUTURE_DUE_MILLIS),
            (
                "Schedule the Delta compliance audit",
                Some(4),
                FUTURE_DUE_MILLIS,
            ),
            (
                "Assign an owner for the partner onboarding kit",
                None,
                OVERDUE_RESPONSE_MILLIS,
            ),
            (
                "Assign an owner for the hosting renewal",
                None,
                FUTURE_DUE_MILLIS,
            ),
        ];
        let mut action_request_ids = Vec::new();
        for (index, (title, owner, due)) in requests.iter().enumerate() {
            let number = index + 1;
            let id = ActionRequestId::parse(format!("demo-action-request-{number}"))
                .map_err(|_| SeedError::Vocabulary("action request id"))?;
            ledger.create_action_request_draft(
                CreateActionRequestDraft {
                    id: id.clone(),
                    title: ActionTitle::parse(*title).map_err(|_| SeedError::Vocabulary("action title"))?,
                    details: ActionDetails::parse("Synthetic request for the Training workspace; accept, decline or reassign it in the app.").map_err(|_| SeedError::Vocabulary("action details"))?,
                    intended_owner: owner.map(|person| stakeholders[person].clone()),
                    response_due_at: Some(UtcTimestamp::from_unix_millis(*due)),
                    intended_action_due_at: Some(UtcTimestamp::from_unix_millis(FUTURE_DUE_MILLIS)),
                    classification: DataClassification::Internal,
                    context: ActionOperationContext {
                        idempotency_id: idempotency(&format!("action-request-create-{number}"))?,
                        correlation_id: correlation(&format!("action-request-{number}"))?,
                    },
                },
                self.audit()?,
                self.now,
            )?;
            ledger.submit_action_request(
                SubmitActionRequest {
                    request_id: id.clone(),
                    expected_version: AggregateVersion::initial(),
                    context: ActionOperationContext {
                        idempotency_id: idempotency(&format!("action-request-submit-{number}"))?,
                        correlation_id: correlation(&format!("action-request-submit-{number}"))?,
                    },
                },
                self.audit()?,
                self.now,
            )?;
            action_request_ids.push(id);
        }
        counts.insert("action_requests".to_owned(), action_request_ids.len());

        let decisions: [(&str, Option<usize>); 3] = [
            ("Choose the Atlas data-residency region", Some(3)),
            ("Decide whether Beacon ships without offline mode", Some(1)),
            ("Decide the hosting vendor renewal term", None),
        ];
        for (index, (subject, owner)) in decisions.iter().enumerate() {
            let number = index + 1;
            let id = DecisionRequestId::parse(format!("demo-decision-request-{number}"))
                .map_err(|_| SeedError::Vocabulary("decision request id"))?;
            ledger.create_decision_request_draft(
                CreateDecisionRequestDraft {
                    id: id.clone(),
                    subject: DecisionSubject::parse(*subject).map_err(|_| SeedError::Vocabulary("decision subject"))?,
                    details: DecisionText::parse("Synthetic decision request; resolve it in the app with a recorded rationale.").map_err(|_| SeedError::Vocabulary("decision text"))?,
                    intended_owner: owner.map(|person| stakeholders[person].clone()),
                    classification: DataClassification::Internal,
                    context: DecisionOperationContext {
                        idempotency_id: idempotency(&format!("decision-request-create-{number}"))?,
                        correlation_id: correlation(&format!("decision-request-{number}"))?,
                    },
                },
                self.audit()?,
                self.now,
            )?;
            ledger.submit_decision_request(
                SubmitDecisionRequest {
                    request_id: id,
                    expected_version: AggregateVersion::initial(),
                    context: DecisionOperationContext {
                        idempotency_id: idempotency(&format!("decision-request-submit-{number}"))?,
                        correlation_id: correlation(&format!("decision-request-submit-{number}"))?,
                    },
                },
                self.audit()?,
                self.now,
            )?;
        }
        counts.insert("decision_requests".to_owned(), decisions.len());

        let risks = [
            "Key vendor concentration",
            "Launch date slips past the partner window",
            "Support backlog exceeds staffing",
            "Compliance finding blocks Delta renewal",
        ];
        for (index, title) in risks.iter().enumerate() {
            let number = index + 1;
            ledger.create_risk(
                CreateRisk {
                    id: RiskId::parse(format!("demo-risk-{number}"))
                        .map_err(|_| SeedError::Vocabulary("risk id"))?,
                    title: RiskTitle::parse(*title)
                        .map_err(|_| SeedError::Vocabulary("risk title"))?,
                    details: RiskDetails::parse("Synthetic risk; record a response in the app.")
                        .map_err(|_| SeedError::Vocabulary("risk details"))?,
                    classification: DataClassification::Internal,
                    context: RiskOperationContext {
                        idempotency_id: idempotency(&format!("risk-create-{number}"))?,
                        correlation_id: correlation(&format!("risk-{number}"))?,
                    },
                },
                self.audit()?,
                self.now,
            )?;
        }
        counts.insert("risks".to_owned(), risks.len());

        let issues = [
            "Nightly export fails on large accounts",
            "Onboarding email lands in spam",
            "Dashboard totals disagree with the ledger export",
            "Nightly export fails on large accounts (second occurrence)",
        ];
        for (index, title) in issues.iter().enumerate() {
            let number = index + 1;
            ledger.create_issue(
                CreateIssue {
                    id: IssueId::parse(format!("demo-issue-{number}"))
                        .map_err(|_| SeedError::Vocabulary("issue id"))?,
                    title: IssueTitle::parse(*title)
                        .map_err(|_| SeedError::Vocabulary("issue title"))?,
                    details: IssueDetails::parse(
                        "Synthetic issue; resolve or close it in the app once verified.",
                    )
                    .map_err(|_| SeedError::Vocabulary("issue details"))?,
                    classification: DataClassification::Internal,
                    // Always None: the SQLite writer refuses `recurrence_of`
                    // outright (issue_repository), so the recurrence attention
                    // flag cannot be seeded.
                    recurrence_of: None,
                    context: IssueOperationContext {
                        idempotency_id: idempotency(&format!("issue-create-{number}"))?,
                        correlation_id: correlation(&format!("issue-{number}"))?,
                    },
                },
                self.audit()?,
                self.now,
            )?;
        }
        counts.insert("issues".to_owned(), issues.len());

        // ---- Evidence: every state the app can show, from real files -----
        let mut evidence = 0;
        let mut links = 0;

        // 1. Pinned and verified against the file's real bytes.
        let pinned_digest = digest_of(&vault_dir.join(VAULT_PINNED))?;
        let pinned = self.create_reference(
            ledger,
            "demo-evidence-1",
            VAULT_PINNED,
            Some(pinned_digest.clone()),
            EvidenceVerification::Verified {
                verified_at: self.now,
                integrity_digest: pinned_digest,
            },
            DataClassification::Internal,
        )?;
        evidence += 1;
        self.link(
            ledger,
            &pinned,
            1,
            EvidenceLinkTarget::Product(products[0].clone()),
            "evidence-1-product-1",
        )?;
        links += 1;

        // 2. Created without a pin, then observed: ObservedUnpinned.
        let unpinned = self.create_reference(
            ledger,
            "demo-evidence-2",
            VAULT_UNPINNED,
            None,
            EvidenceVerification::Unverified,
            DataClassification::Internal,
        )?;
        evidence += 1;
        reobserve_evidence_verification(
            ledger,
            vault,
            &unpinned,
            // Just created above; nothing has advanced it.
            AggregateVersion::initial(),
            idempotency("evidence-2-observe")?,
            correlation("evidence-2-observe")?,
            self.audit()?,
            self.now,
        )?;
        self.link(
            ledger,
            &unpinned,
            2,
            EvidenceLinkTarget::Product(products[1].clone()),
            "evidence-2-product-2",
        )?;
        links += 1;

        // 3. Pinned and verified, then the file disappears: DegradedLastVerified.
        let degraded_path = vault_dir.join(VAULT_DEGRADED);
        let degraded_digest = digest_of(&degraded_path)?;
        let degraded = self.create_reference(
            ledger,
            "demo-evidence-3",
            VAULT_DEGRADED,
            Some(degraded_digest.clone()),
            EvidenceVerification::Verified {
                verified_at: self.now,
                integrity_digest: degraded_digest,
            },
            DataClassification::Internal,
        )?;
        evidence += 1;
        fs::remove_file(&degraded_path)?;
        reobserve_evidence_verification(
            ledger,
            vault,
            &degraded,
            // Just created above; removing the file advances nothing.
            AggregateVersion::initial(),
            idempotency("evidence-3-observe")?,
            correlation("evidence-3-observe")?,
            self.audit()?,
            self.now,
        )?;
        self.link(
            ledger,
            &degraded,
            2,
            EvidenceLinkTarget::Product(products[2].clone()),
            "evidence-3-product-3",
        )?;
        links += 1;

        // 4. Never observed, no file: Unverified.
        let missing = self.create_reference(
            ledger,
            "demo-evidence-4",
            "Research/never-observed.md",
            None,
            EvidenceVerification::Unverified,
            DataClassification::Internal,
        )?;
        evidence += 1;
        self.link(
            ledger,
            &missing,
            1,
            EvidenceLinkTarget::Milestone(milestones[0].clone()),
            "evidence-4-milestone-1",
        )?;
        links += 1;

        // 5. Restricted, pinned and verified: forces Delta's fold in O01.
        let restricted_digest = digest_of(&vault_dir.join(VAULT_RESTRICTED))?;
        let restricted = self.create_reference(
            ledger,
            "demo-evidence-5",
            VAULT_RESTRICTED,
            Some(restricted_digest.clone()),
            EvidenceVerification::Verified {
                verified_at: self.now,
                integrity_digest: restricted_digest,
            },
            DataClassification::Restricted,
        )?;
        evidence += 1;
        self.link(
            ledger,
            &restricted,
            1,
            EvidenceLinkTarget::Product(products[3].clone()),
            "evidence-5-product-4",
        )?;
        links += 1;

        // 6. Linked to an Action Request rather than a Product.
        let request_digest = digest_of(&vault_dir.join(VAULT_REQUEST))?;
        let request_evidence = self.create_reference(
            ledger,
            "demo-evidence-6",
            VAULT_REQUEST,
            Some(request_digest.clone()),
            EvidenceVerification::Verified {
                verified_at: self.now,
                integrity_digest: request_digest,
            },
            DataClassification::Internal,
        )?;
        evidence += 1;
        self.link(
            ledger,
            &request_evidence,
            1,
            EvidenceLinkTarget::ActionRequest(action_request_ids[0].clone()),
            "evidence-6-action-request-1",
        )?;
        links += 1;

        counts.insert("evidence_references".to_owned(), evidence);
        counts.insert("evidence_links".to_owned(), links);
        Ok(counts)
    }

    fn create_reference(
        &mut self,
        ledger: &mut SqliteProductLedger,
        id: &str,
        vault_path: &str,
        pin: Option<IntegrityDigest>,
        verification: EvidenceVerification,
        classification: DataClassification,
    ) -> Result<EvidenceReferenceId, SeedError> {
        let evidence_id =
            EvidenceReferenceId::parse(id).map_err(|_| SeedError::Vocabulary("evidence id"))?;
        ledger.create_evidence_reference(
            CreateEvidenceReference {
                id: evidence_id.clone(),
                vault_path: VaultRelativePath::parse(vault_path)
                    .map_err(|_| SeedError::Vocabulary("vault path"))?,
                fingerprint: pin
                    .map(|digest| EvidenceFingerprint::new(FingerprintAlgorithm::Sha256, digest)),
                verification,
                classification: Some(classification),
                provenance: self.provenance.clone(),
                context: EvidenceContext {
                    idempotency_id: idempotency(&format!("{id}-create"))?,
                    correlation_id: correlation(&format!("{id}-create"))?,
                },
            },
            self.audit()?,
            self.now,
        )?;
        Ok(evidence_id)
    }

    fn link(
        &mut self,
        ledger: &mut SqliteProductLedger,
        evidence_id: &EvidenceReferenceId,
        expected_version: u64,
        target: EvidenceLinkTarget,
        key: &str,
    ) -> Result<(), SeedError> {
        ledger.link_evidence(
            LinkEvidence {
                evidence_id: evidence_id.clone(),
                expected_evidence_version: AggregateVersion::new(expected_version)
                    .map_err(|_| SeedError::Vocabulary("evidence version"))?,
                target,
                context: EvidenceContext {
                    idempotency_id: idempotency(&format!("link-{key}"))?,
                    correlation_id: correlation(&format!("link-{key}"))?,
                },
            },
            self.audit()?,
            self.now,
        )?;
        Ok(())
    }
}

fn digest_of(path: &Path) -> Result<IntegrityDigest, SeedError> {
    let hex_digest =
        compute_sha256_fingerprint(path).map_err(|error| SeedError::Vault(format!("{error:?}")))?;
    IntegrityDigest::parse(hex_digest).map_err(|_| SeedError::Vocabulary("digest"))
}

fn short(value: &str) -> Result<ShortText, SeedError> {
    ShortText::parse(value).map_err(|_| SeedError::Vocabulary("short text"))
}
fn long(value: &str) -> Result<LongText, SeedError> {
    LongText::parse(value).map_err(|_| SeedError::Vocabulary("long text"))
}
fn correlation(key: &str) -> Result<CorrelationId, SeedError> {
    CorrelationId::parse(format!("demo-correlation-{key}"))
        .map_err(|_| SeedError::Vocabulary("correlation id"))
}
fn idempotency(key: &str) -> Result<IdempotencyId, SeedError> {
    IdempotencyId::parse(format!("demo-idempotency-{key}"))
        .map_err(|_| SeedError::Vocabulary("idempotency id"))
}
fn relationship(id: &str) -> Result<RelationshipId, SeedError> {
    RelationshipId::parse(id).map_err(|_| SeedError::Vocabulary("relationship id"))
}
fn portfolio_context(key: &str) -> Result<PortfolioContext, SeedError> {
    Ok(PortfolioContext {
        idempotency_id: idempotency(key)?,
        correlation_id: correlation(key)?,
    })
}
fn delivery_context(key: &str) -> Result<DeliveryContext, SeedError> {
    Ok(DeliveryContext {
        idempotency_id: idempotency(key)?,
        correlation_id: correlation(key)?,
    })
}
fn relationship_context(key: &str) -> Result<RelationshipContext, SeedError> {
    Ok(RelationshipContext {
        idempotency_id: idempotency(key)?,
        correlation_id: correlation(key)?,
    })
}
fn record_name(value: &str) -> Result<RecordName, SeedError> {
    RecordName::parse(value, &correlation("record-name")?)
        .map_err(|_| SeedError::Vocabulary("record name"))
}
fn outcome(value: &str) -> Result<DefinedOutcome, SeedError> {
    DefinedOutcome::parse(value, &correlation("outcome")?)
        .map_err(|_| SeedError::Vocabulary("defined outcome"))
}
fn criteria(value: &str) -> Result<VerificationCriteria, SeedError> {
    VerificationCriteria::parse(value, &correlation("criteria")?)
        .map_err(|_| SeedError::Vocabulary("verification criteria"))
}
