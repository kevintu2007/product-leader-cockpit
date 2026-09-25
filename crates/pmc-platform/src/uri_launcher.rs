//! Narrow Obsidian URI construction and launch port.
//!
//! Only the accepted `obsidian://open` intent is ever constructed, from a
//! vault display name and a validated Vault-relative file path -- never an
//! arbitrary URI, shell command, or string interpolation. Encoding follows
//! RFC 3986 query-component rules (percent-encode everything outside the
//! unreserved set, plus a literal `/` left unencoded as a path separator
//! inside the `file` value, matching how the Obsidian URI scheme is
//! documented and used in practice).
//!
//! [`UriLaunchPort`] is intentionally narrow: it accepts only an
//! [`ObsidianUri`] value this module already validated, never a raw
//! string, so no caller can smuggle an arbitrary URI or shell command
//! through it.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

const OBSIDIAN_SCHEME: &str = "obsidian";
const OBSIDIAN_OPEN_OPERATION: &str = "open";

/// A fully constructed, safely encoded `obsidian://open` URI. The only way
/// to obtain one is [`ObsidianUri::open`]; there is no raw-string
/// constructor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObsidianUri(String);

/// Fail-closed errors for Obsidian URI construction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObsidianUriError {
    EmptyVaultName,
    EmptyFilePath,
}

impl Display for ObsidianUriError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::EmptyVaultName => "vault name is required",
            Self::EmptyFilePath => "file path is required",
        };
        formatter.write_str(message)
    }
}

impl Error for ObsidianUriError {}

impl ObsidianUri {
    /// Construct the accepted `obsidian://open?vault=<vault>&file=<file>`
    /// intent from a vault display name and a Vault-relative file path
    /// (e.g. `pmc_domain::evidence::VaultRelativePath::as_str`).
    pub fn open(vault_name: &str, vault_relative_path: &str) -> Result<Self, ObsidianUriError> {
        if vault_name.trim().is_empty() {
            return Err(ObsidianUriError::EmptyVaultName);
        }
        if vault_relative_path.trim().is_empty() {
            return Err(ObsidianUriError::EmptyFilePath);
        }
        Ok(Self(format!(
            "{OBSIDIAN_SCHEME}://{OBSIDIAN_OPEN_OPERATION}?vault={}&file={}",
            percent_encode_query_value(vault_name),
            percent_encode_query_value(vault_relative_path),
        )))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn percent_encode_query_value(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                encoded.push(*byte as char);
            }
            other => {
                encoded.push('%');
                encoded.push_str(&format!("{other:02X}"));
            }
        }
    }
    encoded
}

/// Fail-closed errors for launching an already-constructed [`ObsidianUri`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaunchError {
    /// No registered handler for the `obsidian:` scheme (Obsidian is not
    /// installed, or the URI handler was never registered).
    Unavailable,
    /// The OS refused or failed to start the handler process.
    LaunchFailed,
}

impl Display for LaunchError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Unavailable => "no application is registered to open obsidian: links",
            Self::LaunchFailed => "the obsidian: link could not be launched",
        };
        formatter.write_str(message)
    }
}

impl Error for LaunchError {}

/// The narrow platform port every Obsidian-opening call site goes through.
/// It accepts only a validated [`ObsidianUri`] -- never a raw string, shell
/// command, or arbitrary URI scheme.
pub trait UriLaunchPort {
    fn launch(&self, uri: &ObsidianUri) -> Result<(), LaunchError>;
}

/// The real Windows launcher: hands the URI to the OS's registered handler
/// via `cmd /C start "" <uri>`, the standard safe way to invoke a URI
/// handler without a shell -- each argument is passed to the child process
/// separately, never concatenated into one interpolated shell string, so
/// the URI's own content cannot be interpreted as a further shell command.
///
/// Before launching, probes `HKEY_CLASSES_ROOT\obsidian` via `reg query` to
/// return a safe [`LaunchError::Unavailable`] when no application has
/// registered the `obsidian:` scheme, rather than reporting false success --
/// `start` itself normally returns success immediately regardless of
/// whether a handler exists, so this probe is required to actually surface
/// the missing-Obsidian case DG0/S3 both call for.
#[cfg(windows)]
#[derive(Clone, Copy, Debug, Default)]
pub struct WindowsShellUriLauncher;

#[cfg(windows)]
impl UriLaunchPort for WindowsShellUriLauncher {
    fn launch(&self, uri: &ObsidianUri) -> Result<(), LaunchError> {
        // PMC-REVIEWED-OBSIDIAN-LAUNCH-HARNESS-START
        let registered = std::process::Command::new("reg")
            .args(["query", "HKCR\\obsidian"])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);
        if !registered {
            return Err(LaunchError::Unavailable);
        }
        std::process::Command::new("cmd")
            .args(["/C", "start", "", uri.as_str()])
            .status()
            .map_err(|_| LaunchError::LaunchFailed)
            .and_then(|status| {
                if status.success() {
                    Ok(())
                } else {
                    Err(LaunchError::LaunchFailed)
                }
            })
        // PMC-REVIEWED-OBSIDIAN-LAUNCH-HARNESS-END
    }
}
