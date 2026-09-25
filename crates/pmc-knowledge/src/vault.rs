//! Product Vault configuration and health.
//!
//! `VaultRoot` is the only way to obtain a validated Product Vault root:
//! there is no raw-path constructor. Resolving it never creates, writes,
//! or scans the Vault -- it only proves the candidate path is safe to
//! treat as an authority boundary (see [`pmc_platform::paths`]).

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};

use pmc_platform::paths::{validate_canonical_root, PathValidationError};

/// A validated, canonical Product Vault root. Never itself a symlink or
/// reparse point, always absolute and in canonical form.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VaultRoot(PathBuf);

impl VaultRoot {
    /// Validate `candidate` as a safe Product Vault root without creating,
    /// writing, or scanning it.
    pub fn validate(candidate: &Path) -> Result<Self, VaultError> {
        let canonical = validate_canonical_root(candidate).map_err(VaultError::InvalidRoot)?;
        Ok(Self(canonical))
    }

    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// The default Obsidian vault display name: the root's own final path
    /// component. Obsidian identifies a vault by this name (unless the
    /// user renamed it inside Obsidian itself, which this module has no
    /// way to observe); callers needing an override should carry one
    /// through configuration rather than relying on this default alone.
    #[must_use]
    pub fn default_identity_name(&self) -> Option<String> {
        self.0
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
    }
}

/// Fail-closed errors for Vault root validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VaultError {
    InvalidRoot(PathValidationError),
}

impl Display for VaultError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRoot(reason) => {
                write!(formatter, "Product Vault root is invalid: {reason}")
            }
        }
    }
}

impl Error for VaultError {}

/// The Product Vault's current observed availability. `Available` proves
/// only that the root currently resolves to a real, accessible directory
/// at observation time -- it is not cached and callers should re-observe
/// before relying on it for a gated transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VaultHealth {
    Available,
    Unavailable,
}

/// Re-observe whether `root` currently resolves to a real, accessible
/// directory. Never fails: an inaccessible or removed Vault is a normal,
/// expected outcome (Degraded Mode), not a hard error.
#[must_use]
pub fn observe_vault_health(root: &VaultRoot) -> VaultHealth {
    match std::fs::symlink_metadata(root.as_path()) {
        Ok(metadata)
            if metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && !is_reparse_point(&metadata) =>
        {
            VaultHealth::Available
        }
        _ => VaultHealth::Unavailable,
    }
}

#[cfg(windows)]
fn is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
const fn is_reparse_point(_: &std::fs::Metadata) -> bool {
    false
}
