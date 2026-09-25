//! Typed identity and policy boundaries for the isolated runtime workspaces.
//!
//! This module deliberately resolves only the two application-owned workspace
//! identities.  It does not create a workspace, open settings, or touch any
//! ledger, vault, credential, index, or backup state.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::settings::ProtectedSettingsRoot;

const WORKSPACES_DIRECTORY: &str = "workspaces";
const TRAINING_DIRECTORY: &str = "training";
const LIVE_DIRECTORY: &str = "live";
/// Where a synthetic seed places the Product Vault it writes, relative to
/// the workspace root. Owned here rather than by the seed tool because two
/// unrelated crates must agree on it -- the tool that writes the Vault and
/// the host that later reads Evidence out of it -- and a duplicated literal
/// would drift silently the first time either moved.
const SYNTHETIC_VAULT_DIRECTORY: &str = "demo-vault";

/// The two runtime profiles are intentionally explicit rather than a boolean
/// or caller-supplied path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceKind {
    Training,
    Live,
}

impl WorkspaceKind {
    const fn directory_name(self) -> &'static str {
        match self {
            Self::Training => TRAINING_DIRECTORY,
            Self::Live => LIVE_DIRECTORY,
        }
    }
}

/// The result of resolving one typed workspace below the protected settings
/// root.  The path is application-derived and cannot be supplied by callers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceIdentity {
    kind: WorkspaceKind,
    root: WorkspaceRoot,
}

impl WorkspaceIdentity {
    /// Resolve the application-owned child path without creating or opening it.
    pub fn resolve(
        protected_root: &ProtectedSettingsRoot,
        kind: WorkspaceKind,
    ) -> Result<Self, WorkspaceError> {
        let parent = protected_root.path();
        validate_protected_root(parent)?;

        let workspaces = parent.join(WORKSPACES_DIRECTORY);
        let candidate = workspaces.join(kind.directory_name());
        if !candidate.starts_with(parent) || candidate == parent {
            return Err(WorkspaceError::WorkspacePathEscapesProtectedRoot);
        }

        validate_existing_components(parent, &candidate)?;
        Ok(Self {
            kind,
            root: WorkspaceRoot(candidate),
        })
    }

    #[must_use]
    pub const fn kind(&self) -> WorkspaceKind {
        self.kind
    }

    #[must_use]
    pub const fn root(&self) -> &WorkspaceRoot {
        &self.root
    }

    /// Return the policy attached to this typed identity.  This is pure and
    /// performs no filesystem, settings, credential, or database operation.
    #[must_use]
    pub fn synthetic_seed_policy(&self) -> SyntheticSeedPolicy {
        SyntheticSeedPolicy {
            identity: self.clone(),
        }
    }

    /// The Product Vault root a synthetic seed writes for this workspace,
    /// or `None` when this workspace has none.
    ///
    /// Only Training has one. A Live Vault root is **user-selected and
    /// explicitly configured** (the Product Vault design),
    /// and no configuration surface exists yet -- S10 Settings owns it. So
    /// `None` here means "this workspace has no Vault the application may
    /// derive", never "the Vault is missing": deriving a Live path by
    /// convention would contradict the Product Vault design and pre-empt the Settings design.
    ///
    /// Pure: the path is application-derived from the already-validated
    /// workspace root and nothing on disk is created, read or validated.
    /// The caller validates it as a Vault root when it is about to be used.
    #[must_use]
    pub fn synthetic_vault_root(&self) -> Option<PathBuf> {
        match self.kind {
            WorkspaceKind::Training => Some(self.root.as_path().join(SYNTHETIC_VAULT_DIRECTORY)),
            WorkspaceKind::Live => None,
        }
    }

    /// Consume a permit only when it was minted for this exact resolved
    /// Training identity.  The permit carries no public path or kind field.
    pub fn accept_synthetic_seed_permit(
        &self,
        permit: SyntheticSeedPermit,
    ) -> Result<(), WorkspaceError> {
        if self.kind != WorkspaceKind::Training || self != &permit.identity {
            return Err(WorkspaceError::SyntheticSeedPermitMismatch);
        }
        Ok(())
    }
}

/// An application-derived workspace root.  No constructor accepts a raw path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceRoot(PathBuf);

impl WorkspaceRoot {
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for WorkspaceRoot {
    fn as_ref(&self) -> &Path {
        self.as_path()
    }
}

/// Synthetic fixture seeding policy for a resolved workspace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyntheticSeedPolicy {
    identity: WorkspaceIdentity,
}

impl SyntheticSeedPolicy {
    /// Authorize a synthetic seed without performing the seed or touching any
    /// runtime authority.  The caller must later pass the typed permit to a
    /// separately authorized seeding operation.
    pub fn authorize(&self) -> Result<SyntheticSeedPermit, WorkspaceError> {
        match self.identity.kind {
            WorkspaceKind::Training => Ok(SyntheticSeedPermit {
                identity: self.identity.clone(),
            }),
            WorkspaceKind::Live => Err(WorkspaceError::LiveSyntheticSeedDenied),
        }
    }
}

/// An opaque proof bound to the exact resolved Training identity that granted it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyntheticSeedPermit {
    identity: WorkspaceIdentity,
}

/// Fail-closed errors for workspace identity and seed-policy resolution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceError {
    InvalidProtectedRoot,
    WorkspacePathEscapesProtectedRoot,
    WorkspacePathContainsLink,
    WorkspacePathOccupied,
    WorkspacePathUnavailable,
    LiveSyntheticSeedDenied,
    SyntheticSeedPermitMismatch,
}

impl Display for WorkspaceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidProtectedRoot => "protected settings root is invalid",
            Self::WorkspacePathEscapesProtectedRoot => {
                "workspace path escapes the protected settings root"
            }
            Self::WorkspacePathContainsLink => "workspace path contains a link or reparse point",
            Self::WorkspacePathOccupied => "workspace path is occupied by a non-directory",
            Self::WorkspacePathUnavailable => "workspace path could not be inspected",
            Self::LiveSyntheticSeedDenied => "synthetic seeding is denied for the Live workspace",
            Self::SyntheticSeedPermitMismatch => {
                "synthetic seed permit belongs to another workspace"
            }
        };
        formatter.write_str(message)
    }
}

impl Error for WorkspaceError {}

fn validate_protected_root(parent: &Path) -> Result<(), WorkspaceError> {
    if !parent.is_absolute()
        || parent
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(WorkspaceError::InvalidProtectedRoot);
    }

    let metadata =
        fs::symlink_metadata(parent).map_err(|_| WorkspaceError::InvalidProtectedRoot)?;
    if metadata.file_type().is_symlink() || is_reparse_point(&metadata) || !metadata.is_dir() {
        return Err(WorkspaceError::InvalidProtectedRoot);
    }

    let canonical = fs::canonicalize(parent).map_err(|_| WorkspaceError::InvalidProtectedRoot)?;
    if canonical != parent {
        return Err(WorkspaceError::InvalidProtectedRoot);
    }
    Ok(())
}

fn validate_existing_components(
    protected_parent: &Path,
    candidate: &Path,
) -> Result<(), WorkspaceError> {
    let mut current = protected_parent.to_path_buf();
    let relative = candidate
        .strip_prefix(protected_parent)
        .map_err(|_| WorkspaceError::WorkspacePathEscapesProtectedRoot)?;
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(WorkspaceError::WorkspacePathEscapesProtectedRoot);
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
                    return Err(WorkspaceError::WorkspacePathContainsLink);
                }
                if !metadata.is_dir() {
                    return Err(WorkspaceError::WorkspacePathOccupied);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(WorkspaceError::WorkspacePathUnavailable),
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
const fn is_reparse_point(_: &fs::Metadata) -> bool {
    false
}
