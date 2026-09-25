//! Raw filesystem observation primitives: streamed SHA-256 content
//! fingerprinting, following the Evidence/Vault safe default: a reviewed
//! cryptographic content hash over raw file bytes with versioned algorithm
//! metadata, never normalized silently.
//!
//! This module only computes a fingerprint over a path the caller has
//! already resolved and validated (see [`crate::paths`]); it has no
//! knowledge of a Vault root, Evidence identity, or Ledger state.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use sha2::{Digest, Sha256};

const READ_BUFFER_SIZE: usize = 64 * 1024;

/// Fail-closed errors for content fingerprinting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilesystemError {
    NotFound,
    NotARegularFile,
    Unreadable,
}

impl Display for FilesystemError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::NotFound => "file does not exist",
            Self::NotARegularFile => "path is a symlink, reparse point, or not a regular file",
            Self::Unreadable => "file could not be read",
        };
        formatter.write_str(message)
    }
}

impl Error for FilesystemError {}

/// Stream `path`'s raw bytes through SHA-256 without loading the whole file
/// into memory, and return the lowercase hex digest (64 characters), the
/// exact form [`pmc_domain`]'s `IntegrityDigest`/`EvidenceFingerprint`
/// expect. Rejects a symlink, reparse point, or non-regular-file target
/// even if [`crate::paths::resolve_contained_path`] already validated every
/// directory component leading to it -- the final component itself gets
/// its own explicit check here too.
pub fn compute_sha256_fingerprint(path: &Path) -> Result<String, FilesystemError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            FilesystemError::NotFound
        } else {
            FilesystemError::Unreadable
        }
    })?;
    if metadata.file_type().is_symlink()
        || crate::paths::is_reparse_point(&metadata)
        || !metadata.is_file()
    {
        return Err(FilesystemError::NotARegularFile);
    }
    let mut file = File::open(path).map_err(|_| FilesystemError::Unreadable)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; READ_BUFFER_SIZE];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| FilesystemError::Unreadable)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
