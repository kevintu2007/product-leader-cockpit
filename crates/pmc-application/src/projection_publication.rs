//! Managed-projection publication orchestration: the one place that joins the Ledger's authoritative snapshot,
//! `pmc-knowledge`'s deterministic generator, and `pmc-platform`'s
//! constrained file publication.
//!
//! Two things live here and nowhere else.
//!
//! First, the **type conversion** between `pmc-knowledge`'s projection
//! vocabulary and `pmc-domain`'s H2a vocabulary. Those two describe the
//! same six record kinds and the same three change kinds, but are defined
//! separately because `pmc-domain` cannot depend on `pmc-knowledge` (the
//! dependency runs the other way). This crate depends on both, so it is
//! the honest place to translate.
//!
//! Second, the **write ordering** ADR 0007 requires. Publication is
//! deliberately NOT done inside a Ledger transaction: a short transaction
//! opens the operation, files are written outside it, and a second short
//! transaction makes it terminal. That is what lets a Ledger commit stay
//! valid when the filesystem half fails, and what keeps `Synchronized` an
//! integrity result rather than a publish-attempt result.

use std::path::Path;

use pmc_domain::managed_projection_rebuild::{
    ManagedProjectionManifestEntry, ManagedProjectionManifestGeneration,
    ManagedProjectionPublishItem, ManagedProjectionPublishItemState,
    ManagedProjectionPublishReport, ManagedProjectionPublishStatus,
    ManagedProjectionRebuildChangeEntry, ManagedProjectionRebuildChangeKind,
    ManagedProjectionRebuildRecordKind,
};
use pmc_knowledge::projections::{
    verify_staged_artifact_set, ProjectionArtifact, ProjectionArtifactChange,
    ProjectionArtifactSet, ProjectionChangeSet, ProjectionChangeSetEntry, ProjectionRecordType,
};
use pmc_platform::filesystem::compute_sha256_fingerprint;
use pmc_platform::managed_publication::{publish_managed_file, remove_managed_file};
use pmc_platform::paths::{resolve_contained_path, validate_canonical_root};

/// Translates one generated record kind into the H2a vocabulary. Total by
/// construction: both enums are closed over the same six kinds, so a new
/// projected record type fails to compile here rather than silently
/// mapping to the wrong one.
#[must_use]
pub const fn record_kind_of(
    record_type: ProjectionRecordType,
) -> ManagedProjectionRebuildRecordKind {
    match record_type {
        ProjectionRecordType::Product => ManagedProjectionRebuildRecordKind::Product,
        ProjectionRecordType::Project => ManagedProjectionRebuildRecordKind::Project,
        ProjectionRecordType::Action => ManagedProjectionRebuildRecordKind::Action,
        ProjectionRecordType::Decision => ManagedProjectionRebuildRecordKind::Decision,
        ProjectionRecordType::Risk => ManagedProjectionRebuildRecordKind::Risk,
        ProjectionRecordType::Kpi => ManagedProjectionRebuildRecordKind::Kpi,
    }
}

#[must_use]
pub const fn change_kind_of(
    change: ProjectionArtifactChange,
) -> ManagedProjectionRebuildChangeKind {
    match change {
        ProjectionArtifactChange::Created => ManagedProjectionRebuildChangeKind::Created,
        ProjectionArtifactChange::Updated => ManagedProjectionRebuildChangeKind::Updated,
        ProjectionArtifactChange::Removed => ManagedProjectionRebuildChangeKind::Removed,
    }
}

/// Converts a generated change set into the H2a change entries a prepared
/// intent binds.
///
/// A `Created`/`Updated` entry carries the freshly generated file's
/// complete-file hash, looked up from `fresh`; a `Removed` entry carries
/// none, because nothing is written for a removal. An entry whose artifact
/// is missing from `fresh` is dropped rather than guessed at -- that
/// combination cannot arise from `diff_against_published`, and inventing a
/// hash would put an unverifiable expectation into an approved preview.
#[must_use]
pub fn h2a_change_entries(
    change_set: &ProjectionChangeSet,
    fresh: &ProjectionArtifactSet,
) -> Vec<ManagedProjectionRebuildChangeEntry> {
    change_set
        .entries
        .iter()
        .filter_map(|entry| {
            let expected_final_file_sha256 = match entry.change {
                ProjectionArtifactChange::Removed => None,
                ProjectionArtifactChange::Created | ProjectionArtifactChange::Updated => {
                    Some(artifact_for(fresh, entry)?.final_file_sha256.clone())
                }
            };
            Some(ManagedProjectionRebuildChangeEntry {
                relative_path: entry.relative_path.clone(),
                record_kind: record_kind_of(entry.record_type),
                record_id: entry.record_id.clone(),
                change: change_kind_of(entry.change),
                classification: entry.classification,
                expected_final_file_sha256,
            })
        })
        .collect()
}

fn artifact_for<'a>(
    fresh: &'a ProjectionArtifactSet,
    entry: &ProjectionChangeSetEntry,
) -> Option<&'a ProjectionArtifact> {
    fresh
        .artifacts
        .iter()
        .find(|artifact| artifact.relative_path == entry.relative_path)
}

/// The manifest generation describing what is now published, built from
/// the freshly generated set. Only ever handed to the Ledger when every
/// planned file committed.
#[must_use]
pub fn manifest_entries_of(fresh: &ProjectionArtifactSet) -> Vec<ManagedProjectionManifestEntry> {
    fresh
        .manifest_entries()
        .into_iter()
        .map(|entry| ManagedProjectionManifestEntry {
            relative_path: entry.relative_path,
            record_kind: record_kind_of(entry.record_type),
            record_id: entry.record_id,
            source_revision: entry.source_revision,
            classification: entry.classification,
            managed_payload_sha256: entry.managed_payload_sha256,
            final_file_sha256: entry.final_file_sha256,
        })
        .collect()
}

/// Every planned path as `Untouched` -- the honest report when publication
/// is refused before it starts, since nothing on disk was touched at all.
fn untouched_items(
    changes: &[ManagedProjectionRebuildChangeEntry],
) -> Vec<ManagedProjectionPublishItem> {
    changes
        .iter()
        .map(|entry| ManagedProjectionPublishItem {
            relative_path: entry.relative_path.clone(),
            state: ManagedProjectionPublishItemState::Untouched,
        })
        .collect()
}

/// Asked at each safe boundary whether the operation should stop.
///
/// ADR 0006 requires cancellation to be defined only where an operation can
/// stop without ambiguous state, and DG0 places this operation's boundary
/// "between exact-path file publications". Modelling the signal as a poll
/// rather than an interrupt is what makes that guarantee real: a file is
/// never abandoned half-written, because the question is only ever asked
/// between whole publications.
///
/// The Ledger implements this by reporting whether the operation row has
/// moved to `cancelling`; tests implement it directly.
pub trait CancellationSignal {
    fn is_cancellation_requested(&self) -> bool;
}

/// A signal that never asks to stop -- for callers with no cancellation
/// affordance, and the honest default for an H1-Auto refresh small enough
/// that ADR 0007 admitted it as automatable in the first place.
#[derive(Clone, Copy, Debug, Default)]
pub struct NeverCancelled;

impl CancellationSignal for NeverCancelled {
    fn is_cancellation_requested(&self) -> bool {
        false
    }
}

/// Publishes an approved change set to the managed subtree, outside any
/// Ledger transaction.
///
/// Stops at the first failure rather than pressing on: ADR 0007 allows
/// cancellation and failure only at enumerated safe boundaries, and the
/// boundary here is between exact-path publications. Every remaining path
/// stays `Untouched`, so the returned report distinguishes what committed,
/// what failed, and what was never attempted -- which is exactly what a
/// repair needs to know. The report never claims more than it observed.
///
/// `staging_token` disambiguates the temporary file each publication
/// stages beside its destination; a caller passes something unique to the
/// operation, such as its idempotency identifier.
#[must_use]
pub fn publish_change_set(
    managed_root: &Path,
    fresh: &ProjectionArtifactSet,
    changes: &[ManagedProjectionRebuildChangeEntry],
    staging_token: &str,
    cancellation: &dyn CancellationSignal,
) -> ManagedProjectionPublishReport {
    // Nothing is written until the whole staged set re-derives from its own
    // allowlisted frontmatter. `ProjectionArtifact::content` is public, so
    // without this the bytes published are whatever the caller put in the
    // struct: the field allowlist would constrain the *generator* while the
    // publisher wrote around it. Re-rendering is what makes the allowlist a
    // property of what lands on disk rather than of one code path.
    if verify_staged_artifact_set(fresh).is_err() {
        return ManagedProjectionPublishReport {
            status: ManagedProjectionPublishStatus::Failed,
            items: untouched_items(changes),
            manifest: None,
        };
    }
    let mut items = Vec::with_capacity(changes.len());
    let mut failed = false;
    let mut cancelled = false;
    for entry in changes {
        // The safe boundary, asked before each whole publication so no file
        // is ever left half-written by a cancellation.
        if !failed && !cancelled && cancellation.is_cancellation_requested() {
            cancelled = true;
        }
        if failed || cancelled {
            items.push(ManagedProjectionPublishItem {
                relative_path: entry.relative_path.clone(),
                state: ManagedProjectionPublishItemState::Untouched,
            });
            continue;
        }
        let relative = Path::new(&entry.relative_path);
        let outcome = match entry.change {
            ManagedProjectionRebuildChangeKind::Removed => {
                remove_managed_file(managed_root, relative).is_ok()
            }
            ManagedProjectionRebuildChangeKind::Created
            | ManagedProjectionRebuildChangeKind::Updated => {
                match fresh
                    .artifacts
                    .iter()
                    .find(|artifact| artifact.relative_path == entry.relative_path)
                {
                    // The approval bound an exact final-file hash for this
                    // path. Publishing bytes that hash to anything else
                    // would make approval advisory: a caller could have a
                    // preview approved and then publish different content
                    // at the same path. The set is already self-verified
                    // above, so comparing the artifact's declared hash to
                    // the approved one closes the chain from approved hash
                    // to allowlisted frontmatter to bytes on disk.
                    Some(artifact)
                        if entry.expected_final_file_sha256.as_deref()
                            == Some(artifact.final_file_sha256.as_str()) =>
                    {
                        publish_managed_file(
                            managed_root,
                            relative,
                            &artifact.content,
                            staging_token,
                        )
                        .is_ok()
                    }
                    // Approved one file, generated another. Fail the path
                    // rather than publishing the unapproved bytes.
                    Some(_) => false,
                    // The approved preview names a file the generator did
                    // not produce. Treat it as a failure rather than
                    // skipping it: publishing less than was approved must
                    // not be reported as success.
                    None => false,
                }
            }
        };
        if outcome {
            items.push(ManagedProjectionPublishItem {
                relative_path: entry.relative_path.clone(),
                state: ManagedProjectionPublishItemState::Committed,
            });
        } else {
            failed = true;
            items.push(ManagedProjectionPublishItem {
                relative_path: entry.relative_path.clone(),
                state: ManagedProjectionPublishItemState::Failed,
            });
        }
    }

    let everything_committed = items
        .iter()
        .all(|item| item.state == ManagedProjectionPublishItemState::Committed);
    // Cancelled and Failed are different facts and ADR 0006 keeps them
    // distinct: a cancelled publication stopped because it was asked to,
    // a failed one because it could not continue. A cancellation that
    // arrived after the last file still committed everything, so it is
    // reported as the success it was.
    let status = if everything_committed {
        ManagedProjectionPublishStatus::Succeeded
    } else if cancelled {
        ManagedProjectionPublishStatus::Cancelled
    } else {
        ManagedProjectionPublishStatus::Failed
    };
    ManagedProjectionPublishReport {
        status,
        items,
        // A manifest is supplied only when the whole set landed. Anything
        // less leaves the previous verified generation as the baseline and
        // the head out-of-sync.
        manifest: None,
    }
}

/// Verifies the managed subtree against the set that was supposed to land
/// and, only if every file on disk matches, attaches the manifest that
/// makes the publication `Synchronized`.
///
/// This is the step that makes "Synchronized" an integrity result rather
/// than a publish-attempt result (ADR 0007). `publish_change_set`
/// deliberately never mints a manifest -- it only knows whether its own
/// writes returned success, which is a claim about the attempt. This
/// function re-reads what is actually on disk and hashes it, so the head
/// only advances on evidence. A publication that succeeded and then had a
/// file changed underneath it does not verify, and correctly reports out of
/// sync rather than synchronized.
///
/// Verification covers the whole fresh set, not just the changed paths: a
/// projection is synchronized when *everything* it should contain is
/// present and correct, and an unchanged file that has since been edited by
/// hand is exactly the manual conflict ADR 0007 wants escalated.
#[must_use]
pub fn attach_verified_manifest(
    managed_root: &Path,
    fresh: &ProjectionArtifactSet,
    report: ManagedProjectionPublishReport,
    manifest_id: String,
) -> ManagedProjectionPublishReport {
    let everything_committed = report.status == ManagedProjectionPublishStatus::Succeeded
        && report
            .items
            .iter()
            .all(|item| item.state == ManagedProjectionPublishItemState::Committed);
    if !everything_committed || !published_set_matches(managed_root, fresh) {
        return ManagedProjectionPublishReport {
            manifest: None,
            ..report
        };
    }
    ManagedProjectionPublishReport {
        manifest: Some(ManagedProjectionManifestGeneration {
            manifest_id,
            ledger_schema_version: fresh.ledger_schema_version,
            ledger_revision: fresh.ledger_revision,
            manifest_digest: fresh.aggregate_manifest_digest(),
            entries: manifest_entries_of(fresh),
        }),
        ..report
    }
}

/// Whether every artifact in the set is present on disk with exactly the
/// bytes it was generated and hashed as.
fn published_set_matches(managed_root: &Path, fresh: &ProjectionArtifactSet) -> bool {
    let Ok(root) = validate_canonical_root(managed_root) else {
        return false;
    };
    fresh.artifacts.iter().all(|artifact| {
        resolve_contained_path(&root, Path::new(&artifact.relative_path))
            .ok()
            .and_then(|path| compute_sha256_fingerprint(&path).ok())
            .is_some_and(|digest| digest == artifact.final_file_sha256)
    })
}
