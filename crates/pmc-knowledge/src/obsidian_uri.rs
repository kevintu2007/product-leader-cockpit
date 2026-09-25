//! Narrow, Vault-identity-aware Obsidian open intent.
//!
//! Combines a validated [`VaultRoot`]'s own display name with a stable
//! [`VaultRelativePath`] to construct and launch exactly one
//! `obsidian://open` intent -- never any other URI or shell command.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use pmc_domain::evidence::VaultRelativePath;
use pmc_platform::uri_launcher::{LaunchError, ObsidianUri, ObsidianUriError, UriLaunchPort};

use crate::vault::VaultRoot;

/// Fail-closed errors for opening an Evidence source in Obsidian.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenInObsidianError {
    /// The Vault root has no final path component to use as its Obsidian
    /// display name (e.g. it is a filesystem root).
    VaultIdentityUnavailable,
    UriConstruction(ObsidianUriError),
    Launch(LaunchError),
}

impl Display for OpenInObsidianError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::VaultIdentityUnavailable => {
                formatter.write_str("Vault root has no usable Obsidian display name")
            }
            Self::UriConstruction(error) => write!(formatter, "{error}"),
            Self::Launch(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for OpenInObsidianError {}

/// Construct and launch the `obsidian://open` intent for one Evidence
/// source. `launcher` is a narrow [`UriLaunchPort`] -- a fake for
/// automated tests, [`pmc_platform::uri_launcher::WindowsShellUriLauncher`]
/// for real Windows use.
pub fn open_in_obsidian<P: UriLaunchPort>(
    vault: &VaultRoot,
    relative_path: &VaultRelativePath,
    launcher: &P,
) -> Result<(), OpenInObsidianError> {
    let vault_name = vault
        .default_identity_name()
        .ok_or(OpenInObsidianError::VaultIdentityUnavailable)?;
    let uri = ObsidianUri::open(&vault_name, relative_path.as_str())
        .map_err(OpenInObsidianError::UriConstruction)?;
    launcher.launch(&uri).map_err(OpenInObsidianError::Launch)
}
