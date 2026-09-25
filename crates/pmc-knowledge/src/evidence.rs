//! Evidence availability/freshness observation.
//!
//! Resolves a stable [`pmc_domain::evidence::VaultRelativePath`] against a
//! validated [`VaultRoot`], computes a content fingerprint when the file
//! exists, and derives the Ledger's own
//! [`pmc_domain::work_management::EvidenceVerification`] state from it --
//! without ever copying a note body into the Ledger or into this crate's
//! own state. The comparison target is the fingerprint pinned at
//! `CreateEvidenceReference` time (`EvidenceReferenceRecord::fingerprint`),
//! not "whatever was last observed", so a changed file is reported as
//! `IntegrityMismatch` rather than silently re-verified against itself.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::Path;

use pmc_domain::evidence::VaultRelativePath;
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::{EvidenceVerification, IntegrityDigest};
use pmc_platform::filesystem::compute_sha256_fingerprint;
use pmc_platform::paths::resolve_contained_path;

use crate::vault::VaultRoot;

/// Fail-closed errors that stop observation entirely -- distinct from a
/// normal "Evidence unavailable" outcome, which is a valid
/// [`EvidenceVerification`] state, not an `Err` here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceObservationError {
    /// The resolved path would escape the Vault root or pass through a
    /// symlink/reparse point -- a security-relevant condition the caller
    /// should treat as exceptional, not a routine "file missing" case.
    PathInvalid,
}

impl Display for EvidenceObservationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("evidence path failed Vault containment validation")
    }
}

impl Error for EvidenceObservationError {}

/// Re-observe one Evidence source against the live filesystem.
///
/// - `expected_fingerprint`: the fingerprint pinned when the Evidence was
///   created, if any (`EvidenceReferenceRecord::fingerprint`). `None` means
///   no fingerprint was pinned, so a successful read yields
///   `ObservedUnpinned` carrying the digest and time -- never `Verified`,
///   because nothing was verified against anything.
/// - `previous`: the Evidence's current stored `EvidenceVerification`,
///   used only to carry forward the true original `verified_at` time when
///   degrading to `DegradedLastVerified` (never to compare against a
///   moving baseline).
pub fn observe_evidence(
    vault: &VaultRoot,
    relative_path: &VaultRelativePath,
    expected_fingerprint: Option<&IntegrityDigest>,
    previous: &EvidenceVerification,
    now: UtcTimestamp,
) -> Result<EvidenceVerification, EvidenceObservationError> {
    let resolved = resolve_contained_path(vault.as_path(), Path::new(relative_path.as_str()))
        .map_err(|_| EvidenceObservationError::PathInvalid)?;
    Ok(match compute_sha256_fingerprint(&resolved) {
        Ok(digest_hex) => match IntegrityDigest::parse(digest_hex) {
            Ok(digest) => match expected_fingerprint {
                None => EvidenceVerification::ObservedUnpinned {
                    observed_at: now,
                    integrity_digest: digest,
                },
                Some(expected) if expected.as_str() == digest.as_str() => {
                    EvidenceVerification::Verified {
                        verified_at: now,
                        integrity_digest: digest,
                    }
                }
                Some(_) => EvidenceVerification::IntegrityMismatch,
            },
            // SHA-256 always produces exactly 64 lowercase hex characters,
            // so IntegrityDigest::parse cannot actually fail here; treat it
            // the same as an unavailable read rather than panic or lie.
            Err(_) => degrade(previous),
        },
        Err(_) => degrade(previous),
    })
}

fn degrade(previous: &EvidenceVerification) -> EvidenceVerification {
    match previous {
        EvidenceVerification::Verified {
            verified_at,
            integrity_digest,
        } => EvidenceVerification::DegradedLastVerified {
            last_verified_at: *verified_at,
            integrity_digest: integrity_digest.clone(),
        },
        EvidenceVerification::DegradedLastVerified { .. } => previous.clone(),
        // An unpinned observation carries no verified baseline to degrade
        // to: when the file cannot be read, all that is known is that it
        // is unreadable.
        EvidenceVerification::Unverified
        | EvidenceVerification::IntegrityMismatch
        | EvidenceVerification::ObservedUnpinned { .. } => EvidenceVerification::Unverified,
    }
}
