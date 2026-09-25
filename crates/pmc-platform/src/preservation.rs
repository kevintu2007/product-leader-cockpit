//! Preservation copies (DG3 restore-unopened amendment §1, accepted
//! 2026-09-23; cut B2 of its design).
//!
//! When the current Ledger cannot be opened, what a restore keeps of it is
//! not an Operational Backup — PMC cannot verify it as a usable Product
//! Ledger — but an exact copy of its files: the Ledger and whichever SQLite
//! sidecars are present, encrypted with the backup passphrase into the backup
//! folder, re-read and matched byte for byte. It is named
//! `pmc-preservation-…`, never `pmc-operational-…`, so the backup registry,
//! its orphan scan and its retention never see it; and its archive profile is
//! `preservation/v1`, so it is never read as an Operational Backup either.
//! Nothing here prunes one.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::backup_archive::{
    read_preservation_archive, write_preservation_archive, ArchiveError, ArchiveMemberSource,
    ManifestInput, ManifestMember, Passphrase,
};
use crate::paths::is_reparse_point;
use crate::restore_control::{FileMemberBinding, FileSetBinding, LedgerFileMember};

pub const PRESERVATION_PREFIX: &str = "pmc-preservation-";
pub const PRESERVATION_SUFFIX: &str = "-v1.tar.zst.age";

/// A member's name inside the archive.
#[must_use]
pub const fn archive_member_name(member: LedgerFileMember) -> &'static str {
    match member {
        LedgerFileMember::Ledger => "ledger",
        LedgerFileMember::Wal => "ledger-wal",
        LedgerFileMember::Shm => "ledger-shm",
    }
}

/// Where a member of `ledger`'s file set lives.
#[must_use]
pub fn member_path(ledger: &Path, member: LedgerFileMember) -> PathBuf {
    let mut name = ledger.as_os_str().to_owned();
    name.push(member.suffix());
    PathBuf::from(name)
}

#[derive(Debug)]
pub enum PreservationError {
    /// The Ledger file is missing, a member is a link or not a regular file,
    /// or a member could not be read.
    Unreadable(io::Error),
    /// The files changed while they were being copied.
    SourceChanged,
    /// The archive could not be written or read back.
    Archive(ArchiveError),
    /// Read back, the copy did not match the files byte for byte.
    VerificationFailed,
    /// The file name is not a preservation copy's, or one by that name
    /// already exists.
    InvalidName,
    /// Another program has one of the Ledger files open (§5).
    Locked,
    Io(io::Error),
}

impl std::fmt::Display for PreservationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(error) => write!(formatter, "ledger files unreadable: {error}"),
            Self::SourceChanged => formatter.write_str("ledger files changed while copied"),
            Self::Archive(error) => write!(formatter, "preservation archive failed: {error}"),
            Self::VerificationFailed => formatter.write_str("preservation copy did not match"),
            Self::InvalidName => formatter.write_str("invalid preservation file name"),
            Self::Locked => formatter.write_str("ledger files are locked by another program"),
            Self::Io(error) => write!(formatter, "preservation I/O failed: {error}"),
        }
    }
}

impl std::error::Error for PreservationError {}

/// Open a member for reading after checking it is a regular file reached
/// through no link. `exclusive` opens it with no sharing at all (Windows),
/// so no other program can write, rename or delete it while it is held.
fn open_member(path: &Path, exclusive: bool) -> io::Result<File> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || is_reparse_point(&metadata) || !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    if exclusive {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    #[cfg(not(windows))]
    let _ = exclusive;
    options.open(path)
}

/// SHA-256 and length of an open member, in one pass.
fn hash_open(file: File) -> io::Result<(u64, String)> {
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut length: u64 = 0;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        length += read as u64;
    }
    Ok((length, format!("{:x}", hasher.finalize())))
}

/// `ledger`'s file set now, bound by content: the Ledger file (required) and
/// each sidecar that is present.
pub fn bind_ledger_files(ledger: &Path) -> Result<FileSetBinding, PreservationError> {
    bind_member_paths(
        &LedgerFileMember::ALL.map(|member| (member, member_path(ledger, member))),
        false,
    )
}

/// [`bind_ledger_files`] with every present member opened with no sharing
/// and all of them held until all are hashed (Windows): the binding is of
/// one moment no other program could write in. A member another program
/// holds is [`PreservationError::Locked`].
pub fn bind_ledger_files_exclusive(ledger: &Path) -> Result<FileSetBinding, PreservationError> {
    bind_member_paths(
        &LedgerFileMember::ALL.map(|member| (member, member_path(ledger, member))),
        true,
    )
}

/// The file set found at `paths` — each member where the caller says it is
/// now, such as the names a restore moved them to.
pub fn bind_member_paths(
    paths: &[(LedgerFileMember, PathBuf)],
    exclusive: bool,
) -> Result<FileSetBinding, PreservationError> {
    let mut held = Vec::new();
    for (member, path) in paths {
        match open_member(path, exclusive) {
            Ok(file) => held.push((*member, file)),
            Err(error)
                if error.kind() == io::ErrorKind::NotFound
                    && *member != LedgerFileMember::Ledger => {}
            // ERROR_SHARING_VIOLATION: another program has it open.
            Err(error) if error.raw_os_error() == Some(32) => {
                return Err(PreservationError::Locked)
            }
            Err(error) => return Err(PreservationError::Unreadable(error)),
        }
    }
    let mut members = Vec::new();
    for (member, file) in held {
        let (length, sha256) = hash_open(file).map_err(PreservationError::Unreadable)?;
        members.push(FileMemberBinding {
            member,
            length,
            sha256,
        });
    }
    FileSetBinding::new(members).ok_or(PreservationError::Unreadable(io::Error::new(
        io::ErrorKind::NotFound,
        "no ledger file",
    )))
}

/// Whether an archive's members are exactly `files`, member for member.
fn matches(members: &[ManifestMember], files: &FileSetBinding) -> bool {
    members.len() == files.members().len()
        && files.members().iter().all(|bound| {
            members.iter().any(|member| {
                member.name == archive_member_name(bound.member)
                    && member.size == bound.length
                    && member.sha256 == bound.sha256
            })
        })
}

/// A preservation copy made and verified.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreservedCopy {
    pub preservation_id: String,
    pub file_name: String,
    /// The files it holds.
    pub files: FileSetBinding,
}

/// Is this a preservation copy's file name?
#[must_use]
pub fn is_preservation_file_name(name: &str) -> bool {
    name.starts_with(PRESERVATION_PREFIX)
        && name.ends_with(PRESERVATION_SUFFIX)
        && name.len() > PRESERVATION_PREFIX.len() + PRESERVATION_SUFFIX.len()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-.".contains(&byte))
}

/// Copy `ledger`'s file set into `destination/file_name`, encrypted with
/// `passphrase`, then read it back and match every member against the files
/// as they were bound. Only then is it returned; a copy that does not match
/// is removed and refused. Never overwrites a file.
pub fn preserve_ledger_files(
    ledger: &Path,
    destination: &Path,
    passphrase: &Passphrase,
    preservation_id: &str,
    created_at: &str,
    file_name: &str,
) -> Result<PreservedCopy, PreservationError> {
    preserve_with(
        ledger,
        destination,
        passphrase,
        preservation_id,
        created_at,
        file_name,
        &|| {},
    )
}

/// [`preserve_ledger_files`], with `after_write` run between writing the copy
/// and checking the file set again (the tests' way to change it there).
#[allow(clippy::too_many_arguments)]
fn preserve_with(
    ledger: &Path,
    destination: &Path,
    passphrase: &Passphrase,
    preservation_id: &str,
    created_at: &str,
    file_name: &str,
    after_write: &dyn Fn(),
) -> Result<PreservedCopy, PreservationError> {
    if !is_preservation_file_name(file_name) {
        return Err(PreservationError::InvalidName);
    }
    let target = destination.join(file_name);
    let staging = destination.join(format!(".{file_name}.partial"));
    if target.exists() {
        return Err(PreservationError::InvalidName);
    }
    let files = bind_ledger_files(ledger)?;
    let paths: Vec<(LedgerFileMember, PathBuf)> = files
        .members()
        .iter()
        .map(|bound| (bound.member, member_path(ledger, bound.member)))
        .collect();
    let sources: Vec<ArchiveMemberSource<'_>> = paths
        .iter()
        .map(|(member, path)| ArchiveMemberSource {
            name: archive_member_name(*member),
            path,
        })
        .collect();
    let input = ManifestInput {
        archive_id: preservation_id.to_owned(),
        created_at: created_at.to_owned(),
        ledger_schema_version: 0,
        ledger_revision: 0,
        snapshot_checksum: files.aggregate_sha256().to_owned(),
    };

    let result = (|| {
        // A staging file of this exact name is only ever an earlier attempt's
        // leftover (a crash before its rename): it was never verified, and it
        // must not stop this one.
        match fs::remove_file(&staging) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(PreservationError::Io(error)),
        }
        let output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)
            .map_err(PreservationError::Io)?;
        let written = write_preservation_archive(&output, passphrase, &input, &sources)
            .map_err(PreservationError::Archive)?;
        output.sync_all().map_err(PreservationError::Io)?;
        drop(output);
        after_write();
        // What was written is the files as bound, and the file set is still
        // exactly that set: a sidecar that appeared, disappeared or changed
        // while the copy was made would otherwise be silently left out.
        if !matches(&written.manifest.members, &files) || bind_ledger_files(ledger)? != files {
            return Err(PreservationError::SourceChanged);
        }
        fs::rename(&staging, &target).map_err(PreservationError::Io)?;
        // Read back from the folder, to its authenticated end.
        let reread = File::open(&target).map_err(PreservationError::Io)?;
        let verified = read_preservation_archive(BufReader::new(reread), passphrase, None)
            .map_err(|_| PreservationError::VerificationFailed)?;
        if !matches(&verified.manifest.members, &files)
            || verified.manifest.snapshot_checksum != files.aggregate_sha256()
            || verified.manifest.archive_id != preservation_id
        {
            return Err(PreservationError::VerificationFailed);
        }
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&staging);
        // A copy that did not verify is not recovery evidence; keeping it
        // would invite trusting it.
        if !matches!(error, PreservationError::InvalidName) {
            let _ = fs::remove_file(&target);
        }
        return Err(error);
    }
    Ok(PreservedCopy {
        preservation_id: preservation_id.to_owned(),
        file_name: file_name.to_owned(),
        files,
    })
}

/// Read a preservation copy back and match it against `files` (before a
/// restore relies on it, and in tests). `false` for any difference.
#[must_use]
pub fn preservation_copy_matches(
    destination: &Path,
    copy: &PreservedCopy,
    passphrase: &Passphrase,
) -> bool {
    File::open(destination.join(&copy.file_name))
        .ok()
        .and_then(|file| read_preservation_archive(BufReader::new(file), passphrase, None).ok())
        .is_some_and(|verified| {
            matches(&verified.manifest.members, &copy.files)
                && verified.manifest.snapshot_checksum == copy.files.aggregate_sha256()
                && verified.manifest.archive_id == copy.preservation_id
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    static SEQUENCE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    fn scratch(name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let directory =
            std::env::temp_dir().join(format!("pmc-preserve-unit-{name}-{unique}-{sequence}"));
        assert!(fs::create_dir_all(&directory).is_ok(), "scratch");
        directory
    }

    const NAME: &str = "pmc-preservation-20260923T010203000Z-u1u1u1u1-v1.tar.zst.age";

    fn passphrase() -> Passphrase {
        Passphrase::new("correct horse battery staple river".to_owned())
    }

    fn names(folder: &Path) -> Vec<String> {
        fs::read_dir(folder)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn a_sidecar_that_appears_while_the_copy_is_made_refuses_the_copy() {
        let work = scratch("race");
        let folder = scratch("race-folder");
        let ledger = work.join("product-ledger.sqlite3");
        assert!(fs::write(&ledger, b"not a sqlite file").is_ok());
        let wal = member_path(&ledger, LedgerFileMember::Wal);
        let appeared = preserve_with(
            &ledger,
            &folder,
            &passphrase(),
            "u1u1u1u1",
            "2026-09-23T01:02:03.000Z",
            NAME,
            &|| {
                let _ = fs::write(&wal, b"late frames");
            },
        );
        assert!(matches!(appeared, Err(PreservationError::SourceChanged)));
        // Nothing unverified is left in the folder.
        assert!(names(&folder).is_empty(), "{:?}", names(&folder));
    }

    #[test]
    fn a_leftover_staging_file_does_not_stop_the_next_attempt() {
        let work = scratch("stale");
        let folder = scratch("stale-folder");
        let ledger = work.join("product-ledger.sqlite3");
        assert!(fs::write(&ledger, b"not a sqlite file").is_ok());
        assert!(fs::write(folder.join(format!(".{NAME}.partial")), b"crashed").is_ok());
        let copy = preserve_ledger_files(
            &ledger,
            &folder,
            &passphrase(),
            "u1u1u1u1",
            "2026-09-23T01:02:03.000Z",
            NAME,
        );
        assert!(copy.is_ok(), "{copy:?}");
        assert_eq!(names(&folder), vec![NAME.to_owned()]);
    }
}
