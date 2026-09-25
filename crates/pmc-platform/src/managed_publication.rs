//! Constrained publication primitives for the managed projection subtree.
//!
//! This is deliberately **not** a general file writer. Every operation is
//! confined to one caller-supplied managed root by
//! [`crate::paths::resolve_contained_path`], which admits only ordinary
//! path components -- no `..`, no absolute path, no prefix. The foundation's
//! stop conditions name "a generic file/shell command" as a reason to halt
//! execution, so the surface here is exactly two verbs (publish one file,
//! remove one file) over relative paths the projection generator produced,
//! and nothing else.
//!
//! Publication replaces atomically: the bytes are written to a temporary
//! file beside the destination, flushed, and then renamed over it, so a
//! crash mid-write can never leave a half-written managed file that would
//! later hash as a manual edit.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::paths::{resolve_contained_path, validate_canonical_root};

/// Fail-closed errors for managed publication. Deliberately coarse: a
/// caller reports these per path as `failed`, and none of them carries the
/// absolute path, which must not reach a safe error or log.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedPublicationError {
    /// The relative path would escape the managed root, or is not a plain
    /// relative path.
    PathEscapesManagedRoot,
    /// The managed root itself is not a usable authority boundary: not an
    /// absolute canonical directory, or itself a symlink or reparse point.
    /// Confining a path to a root that is itself a link would confine it to
    /// wherever that link points, so this is refused before any resolution.
    ManagedRootNotAnAuthorityBoundary,
    /// The destination exists but is a symlink or reparse point. Replacing
    /// it would write through to wherever it points, outside the managed
    /// subtree.
    DestinationNotARegularFile,
    /// The write, flush, rename, or removal failed.
    PublicationFailed,
}

impl Display for ManagedPublicationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PathEscapesManagedRoot => "path escapes the managed projection root",
            Self::ManagedRootNotAnAuthorityBoundary => {
                "managed projection root is not a canonical directory"
            }
            Self::DestinationNotARegularFile => {
                "managed destination is a symlink, reparse point, or not a regular file"
            }
            Self::PublicationFailed => "managed file could not be published",
        })
    }
}

impl Error for ManagedPublicationError {}

fn managed_destination(
    managed_root: &Path,
    relative: &Path,
) -> Result<PathBuf, ManagedPublicationError> {
    // `resolve_contained_path` documents an *already-validated* canonical
    // root as its precondition, and it only inspects the components it
    // appends. Establishing that precondition here rather than trusting the
    // caller is what makes confinement a property of this module: a root
    // that is itself a symlink or junction would otherwise confine every
    // path to wherever it points, with each appended component checked and
    // every one of them innocent.
    let root = validate_canonical_root(managed_root)
        .map_err(|_| ManagedPublicationError::ManagedRootNotAnAuthorityBoundary)?;
    resolve_contained_path(&root, relative)
        .map_err(|_| ManagedPublicationError::PathEscapesManagedRoot)
}

/// Rejects replacing or deleting through a symlink/reparse point. An
/// absent destination is fine -- that is an ordinary create.
fn ensure_replaceable(destination: &Path) -> Result<(), ManagedPublicationError> {
    match fs::symlink_metadata(destination) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink()
                || crate::paths::is_reparse_point(&metadata)
                || !metadata.is_file()
            {
                Err(ManagedPublicationError::DestinationNotARegularFile)
            } else {
                Ok(())
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(ManagedPublicationError::PublicationFailed),
    }
}

/// Writes one managed file atomically, creating any missing directory
/// inside the managed root. `contents` is the complete final file, exactly
/// as the generator produced and hashed it.
pub fn publish_managed_file(
    managed_root: &Path,
    relative: &Path,
    contents: &[u8],
    temporary_suffix: &str,
) -> Result<(), ManagedPublicationError> {
    let destination = managed_destination(managed_root, relative)?;
    ensure_replaceable(&destination)?;
    let parent = destination
        .parent()
        .ok_or(ManagedPublicationError::PathEscapesManagedRoot)?;
    fs::create_dir_all(parent).map_err(|_| ManagedPublicationError::PublicationFailed)?;

    let mut staged = destination.clone().into_os_string();
    staged.push(".");
    staged.push(temporary_suffix);
    staged.push(".tmp");
    let staged = PathBuf::from(staged);

    let write = || -> std::io::Result<()> {
        let mut file = open_stage(&staged)?;
        file.write_all(contents)?;
        // Flush the contents before the rename so the published name can
        // never resolve to a partially written file after a crash.
        file.sync_all()
    };
    if write().is_err() {
        let _ = fs::remove_file(&staged);
        return Err(ManagedPublicationError::PublicationFailed);
    }
    if fs::rename(&staged, &destination).is_err() {
        let _ = fs::remove_file(&staged);
        return Err(ManagedPublicationError::PublicationFailed);
    }
    Ok(())
}

/// Opens the stage file so that nothing already at its path is followed or
/// truncated.
///
/// The stage name is predictable (`<destination>.<token>.tmp`), so something
/// planted there before publication would, with a plain `File::create`, be
/// opened through: a symlink or reparse point pointing outside the managed
/// root would receive the bytes, and an existing file would be truncated.
/// `create_new` refuses any existing path, and on Windows
/// `FILE_FLAG_OPEN_REPARSE_POINT` stops the final component from being
/// followed, so a reparse point at the stage path is seen as existing rather
/// than resolved to its target. On Unix `O_CREAT|O_EXCL` already fails on an
/// existing symlink regardless of where it points.
///
/// One leftover a crashed run can legitimately leave is a plain regular
/// stage file from the same token. That is replaced -- after confirming it
/// is a regular file and not a link -- so a retry can finish. Anything else
/// at the path is refused. `symlink_metadata` never reports a symlink or a
/// junction as a file on either platform, so `is_file` alone excludes links;
/// the reparse-point check is for a file that carries some other reparse
/// tag, which is not understood and therefore not deleted. A hard link
/// planted at the stage path is a regular file: `remove_file` unlinks only
/// that name, so its target is untouched, where `File::create` would have
/// truncated it.
///
/// This narrows the Vault path TOCTOU race and is stated as hardening, not
/// closure: the parent directory race remains, because Rust `std` cannot
/// create a file relative to a directory handle on Windows.
fn open_stage(staged: &Path) -> std::io::Result<File> {
    match open_stage_exclusive(staged) {
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = fs::symlink_metadata(staged)?;
            if !existing.is_file() || crate::paths::is_reparse_point(&existing) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "stage path is occupied by a link, reparse point, or non-regular file",
                ));
            }
            fs::remove_file(staged)?;
            open_stage_exclusive(staged)
        }
        other => other,
    }
}

fn open_stage_exclusive(staged: &Path) -> std::io::Result<File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options.open(staged)
}

/// Removes one managed file. A path that is already absent succeeds: a
/// removal that has to be retried after a partial publication must be able
/// to complete rather than failing on work it already did.
pub fn remove_managed_file(
    managed_root: &Path,
    relative: &Path,
) -> Result<(), ManagedPublicationError> {
    let destination = managed_destination(managed_root, relative)?;
    ensure_replaceable(&destination)?;
    match fs::remove_file(&destination) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(ManagedPublicationError::PublicationFailed),
    }
}
