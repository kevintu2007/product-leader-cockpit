//! General-purpose canonical filesystem path validation:
//! reject a non-absolute or non-canonical root, and reject any symlink or
//! Windows reparse point either at the root itself or along any existing
//! component of a path resolved underneath it.
//!
//! This generalizes the same technique `workspace.rs` established for the
//! application's own protected settings root, for the Product Vault root,
//! which is an arbitrary user-selected location -- not nested under any
//! application-owned protected root, so it needs its own entry point that
//! does not depend on [`crate::settings::ProtectedSettingsRoot`].

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Fail-closed errors for canonical-root and contained-path resolution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathValidationError {
    RootNotAbsolute,
    RootIsLinkOrReparsePoint,
    RootIsNotADirectory,
    RootUnavailable,
    RootNotCanonical,
    PathIsEmpty,
    PathEscapesRoot,
    PathContainsLinkOrReparsePoint,
    PathOccupiedByNonDirectory,
    PathUnavailable,
    /// The chosen entry is not a regular file.
    PathIsNotAFile,
}

impl Display for PathValidationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::RootNotAbsolute => "root path is not an absolute, canonical-form path",
            Self::RootIsLinkOrReparsePoint => "root path is a symlink or reparse point",
            Self::RootIsNotADirectory => "root path is not a directory",
            Self::RootUnavailable => "root path could not be inspected",
            Self::RootNotCanonical => "root path is not in canonical form",
            Self::PathIsEmpty => "relative path has no components",
            Self::PathEscapesRoot => "relative path escapes the root",
            Self::PathContainsLinkOrReparsePoint => {
                "relative path contains a symlink or reparse point"
            }
            Self::PathOccupiedByNonDirectory => {
                "relative path is occupied by a non-directory before its final component"
            }
            Self::PathUnavailable => "relative path could not be inspected",
            Self::PathIsNotAFile => "chosen path is not a regular file",
        };
        formatter.write_str(message)
    }
}

impl Error for PathValidationError {}

/// Validate that `root` is safe to treat as an authority boundary: absolute,
/// free of `.`/`..` components, not itself a symlink/reparse point, a real
/// directory, and already in canonical form. Returns the canonical root.
pub fn validate_canonical_root(root: &Path) -> Result<PathBuf, PathValidationError> {
    if !root.is_absolute()
        || root
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(PathValidationError::RootNotAbsolute);
    }
    let metadata = fs::symlink_metadata(root).map_err(|_| PathValidationError::RootUnavailable)?;
    if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
        return Err(PathValidationError::RootIsLinkOrReparsePoint);
    }
    if !metadata.is_dir() {
        return Err(PathValidationError::RootIsNotADirectory);
    }
    let canonical = fs::canonicalize(root).map_err(|_| PathValidationError::RootUnavailable)?;
    if canonical != root {
        return Err(PathValidationError::RootNotCanonical);
    }
    Ok(canonical)
}

/// Resolve `relative` (a pure relative path made only of normal components --
/// no `.`, `..`, or absolute prefix) underneath an already-validated
/// `canonical_root`, rejecting a symlink or reparse point at any existing
/// component. A component that does not exist yet is accepted for the
/// final (file) component and every component after it would have been --
/// callers decide what "missing" means (e.g. Evidence marked Unavailable);
/// only an existing non-directory before the final component is rejected.
pub fn resolve_contained_path(
    canonical_root: &Path,
    relative: &Path,
) -> Result<PathBuf, PathValidationError> {
    let mut names = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(name) => names.push(name),
            _ => return Err(PathValidationError::PathEscapesRoot),
        }
    }
    if names.is_empty() {
        return Err(PathValidationError::PathIsEmpty);
    }
    let last_index = names.len() - 1;
    let mut current = canonical_root.to_path_buf();
    // Once a component is found missing, every remaining component (nested
    // under something that does not exist yet) cannot itself be an existing
    // symlink/reparse point either, so validation stops there -- but the
    // full intended path is still built and returned, leaving the
    // existence/availability decision to the caller.
    let mut missing = false;
    for (index, name) in names.iter().enumerate() {
        current.push(name);
        if missing {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
                    return Err(PathValidationError::PathContainsLinkOrReparsePoint);
                }
                if index != last_index && !metadata.is_dir() {
                    return Err(PathValidationError::PathOccupiedByNonDirectory);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => missing = true,
            Err(_) => return Err(PathValidationError::PathUnavailable),
        }
    }
    if !current.starts_with(canonical_root) {
        return Err(PathValidationError::PathEscapesRoot);
    }
    Ok(current)
}

/// Evidence from a file (item ⑦-3; DG3 Vault-root and Evidence-from-file
/// amendment §4.1): where a file the person chose lives inside the Vault,
/// as the Vault-relative path the Ledger stores — its components joined
/// with `/`, spelled as the file system spells them (both paths are
/// canonicalized, which on Windows returns each name as it is stored).
///
/// Refused when the file is outside `canonical_root`, when the root or any
/// component from the root down to the file is a link or reparse point, or
/// when the chosen entry is not a regular file. Containment compares path
/// components, never string prefixes, so `C:\Vault2` is not inside
/// `C:\Vault`.
pub fn vault_relative_file(
    canonical_root: &Path,
    chosen: &Path,
) -> Result<String, PathValidationError> {
    let root = validate_canonical_root(canonical_root)?;
    // Walk the chosen path as given first: canonicalize would follow a link
    // and hide it.
    let chosen_metadata =
        fs::symlink_metadata(chosen).map_err(|_| PathValidationError::PathUnavailable)?;
    if chosen_metadata.file_type().is_symlink() || is_reparse_point(&chosen_metadata) {
        return Err(PathValidationError::PathContainsLinkOrReparsePoint);
    }
    if !chosen_metadata.is_file() {
        return Err(PathValidationError::PathIsNotAFile);
    }
    let canonical = fs::canonicalize(chosen).map_err(|_| PathValidationError::PathUnavailable)?;
    let relative = canonical
        .strip_prefix(&root)
        .map_err(|_| PathValidationError::PathEscapesRoot)?;
    let mut names = Vec::new();
    let mut current = root.clone();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(PathValidationError::PathEscapesRoot);
        };
        current.push(name);
        let metadata =
            fs::symlink_metadata(&current).map_err(|_| PathValidationError::PathUnavailable)?;
        if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
            return Err(PathValidationError::PathContainsLinkOrReparsePoint);
        }
        names.push(
            name.to_str()
                .ok_or(PathValidationError::PathUnavailable)?
                .to_owned(),
        );
    }
    if names.is_empty() {
        return Err(PathValidationError::PathIsEmpty);
    }
    Ok(names.join("/"))
}

#[cfg(windows)]
pub(crate) fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
pub(crate) const fn is_reparse_point(_: &fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    static SEQUENCE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    /// A canonical scratch root, as a configured Vault root is.
    fn scratch_root(name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let directory =
            std::env::temp_dir().join(format!("pmc-vault-file-{name}-{unique}-{sequence}"));
        assert!(fs::create_dir_all(&directory).is_ok(), "scratch directory");
        match fs::canonicalize(&directory) {
            Ok(canonical) => canonical,
            Err(error) => panic!("canonical scratch: {error}"),
        }
    }

    fn write(path: &Path) {
        if let Some(parent) = path.parent() {
            assert!(fs::create_dir_all(parent).is_ok(), "parent");
        }
        assert!(fs::write(path, b"synthetic evidence").is_ok(), "file");
    }

    #[test]
    fn a_file_inside_the_vault_is_named_by_its_components() {
        let root = scratch_root("inside");
        let file = root.join("Board").join("2026 Q3").join("會議紀錄.md");
        write(&file);
        assert_eq!(
            vault_relative_file(&root, &file),
            Ok("Board/2026 Q3/會議紀錄.md".to_owned())
        );
    }

    #[cfg(windows)]
    #[test]
    fn the_name_is_spelled_as_windows_stores_it() {
        // Chosen in another case, stored as the file system spells it, so
        // one file never gets two spellings from this path.
        let root = scratch_root("case");
        write(&root.join("Reports").join("Q3.md"));
        let chosen = root.join("REPORTS").join("q3.MD");
        assert_eq!(
            vault_relative_file(&root, &chosen),
            Ok("Reports/Q3.md".to_owned())
        );
    }

    #[test]
    fn a_file_outside_the_vault_is_refused() {
        let root = scratch_root("outside");
        let vault = root.join("Vault");
        let sibling = root.join("Vault2");
        assert!(fs::create_dir_all(&vault).is_ok(), "vault");
        write(&sibling.join("note.md"));
        let vault = match fs::canonicalize(&vault) {
            Ok(vault) => vault,
            Err(error) => panic!("{error}"),
        };
        // A sibling whose name starts with the Vault's is not inside it.
        assert_eq!(
            vault_relative_file(&vault, &sibling.join("note.md")),
            Err(PathValidationError::PathEscapesRoot)
        );
        // Nor is anything reached by climbing out of it.
        assert_eq!(
            vault_relative_file(&vault, &vault.join("..").join("Vault2").join("note.md")),
            Err(PathValidationError::PathEscapesRoot)
        );
    }

    #[test]
    fn only_a_regular_file_can_be_chosen() {
        let root = scratch_root("kinds");
        assert!(fs::create_dir_all(root.join("folder")).is_ok(), "folder");
        assert_eq!(
            vault_relative_file(&root, &root.join("folder")),
            Err(PathValidationError::PathIsNotAFile)
        );
        assert_eq!(
            vault_relative_file(&root, &root.join("missing.md")),
            Err(PathValidationError::PathUnavailable)
        );
        // The root itself is not a file inside it.
        assert_eq!(
            vault_relative_file(&root, &root),
            Err(PathValidationError::PathIsNotAFile)
        );
    }
}
