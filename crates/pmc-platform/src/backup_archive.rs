//! The encrypted Operational Backup archive of ADR 0010.
//!
//! One file: a deterministic tar stream, compressed with zstd, encrypted with
//! age v1 under the scrypt passphrase recipient. No PMC header is added, so
//! the standard tools recover it without PMC:
//! `age -d archive.tar.zst.age | zstd -d | tar -x`.
//!
//! The tar holds the members in the order given and then `manifest.json`,
//! which records every member's name, size and SHA-256. Reading verifies the
//! whole stream to its authenticated end, refuses any entry that is not a
//! plain regular file with a plain name, and compares every member against the
//! manifest; nothing is reported valid before all of that has passed.
//!
//! This module owns only the container. What goes in it (the S2 snapshot, the
//! settings in scope) and where it is published are the caller's.

use std::collections::HashSet;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, Read, Write};
use std::path::Path;

use age::secrecy::SecretString;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// scrypt work factor for new archives: N = 2^18, about 256 MiB (ADR 0010).
pub const PRODUCTION_WORK_FACTOR: u8 = 18;
/// The highest work factor a restore accepts before allocating (ADR 0010).
pub const MAX_ACCEPTED_WORK_FACTOR: u8 = 20;

const ARCHIVE_FORMAT: &str = "pmc-backup/v1";
const MANIFEST_FORMAT: &str = "pmc-backup-manifest/v1";
const OPERATIONAL_PROFILE: &str = "operational/v1";
/// A preservation copy (DG3 restore-unopened amendment §1): the same
/// container, another profile, so neither is ever read as the other.
const PRESERVATION_PROFILE: &str = "preservation/v1";
const MANIFEST_NAME: &str = "manifest.json";
const MAX_MEMBER_NAME: usize = 64;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const ZSTD_LEVEL: i32 = 3;

// Resource ceilings, checked before the manifest is seen, so an archive made
// with the right passphrase but hostile contents cannot exhaust memory or
// disk. An Operational Backup is a Ledger snapshot and a few small files.
const MAX_MEMBERS: usize = 16;
const MAX_ARCHIVE_PLAINTEXT_BYTES: u64 = 16 * 1024 * 1024 * 1024;
/// tar headers and padding on top of the members themselves.
const TAR_OVERHEAD_BYTES: u64 = 1024 * 1024;
const MAX_MANIFEST_FIELD_BYTES: usize = 128;
/// zstd window: 2^27 = 128 MiB, the library's own default ceiling, made
/// explicit so a hostile frame cannot ask for more.
const ZSTD_WINDOW_LOG_MAX: u32 = 27;

/// A recovery passphrase. Never printed; the memory is cleared when dropped.
pub struct Passphrase(SecretString);

impl Passphrase {
    #[must_use]
    pub fn new(passphrase: String) -> Self {
        Self(SecretString::from(passphrase))
    }

    fn secret(&self) -> SecretString {
        use age::secrecy::ExposeSecret;
        SecretString::from(self.0.expose_secret().to_owned())
    }
}

impl fmt::Debug for Passphrase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Passphrase([redacted])")
    }
}

/// One file to put in the archive, under a plain name.
#[derive(Clone, Copy, Debug)]
pub struct ArchiveMemberSource<'a> {
    pub name: &'a str,
    pub path: &'a Path,
}

/// What the caller states about the archive; the member list is computed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestInput {
    pub archive_id: String,
    pub created_at: String,
    pub ledger_schema_version: u32,
    pub ledger_revision: u64,
    pub snapshot_checksum: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestMember {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

/// The manifest inside every archive (`pmc-backup-manifest/v1`).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveManifest {
    pub format: String,
    pub manifest_format: String,
    pub profile: String,
    pub archive_id: String,
    pub created_at: String,
    pub ledger_schema_version: u32,
    pub ledger_revision: u64,
    pub snapshot_checksum: String,
    pub members: Vec<ManifestMember>,
}

/// A finished archive: its manifest and the S7 container checksum.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WrittenArchive {
    pub manifest: ArchiveManifest,
    /// SHA-256 of the complete `.age` bytes, lower-case hex.
    pub container_sha256: String,
    pub container_bytes: u64,
}

/// An archive read to its authenticated end with every member verified.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedArchive {
    pub manifest: ArchiveManifest,
}

#[derive(Debug)]
pub enum ArchiveError {
    /// A member name is not a plain lower-case file name, repeats, or is the
    /// manifest's own name.
    InvalidMember,
    /// Not an age passphrase file.
    NotAnArchive,
    /// The key could not be unwrapped: the passphrase is wrong, or the age
    /// header is damaged. Only a container checksum verified beforehand tells
    /// the two apart.
    CannotDecrypt,
    /// The archive asks for more scrypt work than this reader allows.
    WorkFactorTooHigh,
    /// The ciphertext or compressed stream is damaged or truncated.
    Corrupt,
    /// An entry that is not a plain regular file with a plain name, a repeat,
    /// or anything after the manifest.
    UnexpectedEntry,
    /// The manifest is missing, malformed, from another format, or does not
    /// match what the archive holds.
    ManifestMismatch,
    /// Beyond a resource ceiling: too many members, a member or the whole
    /// archive too large, or a manifest field too long.
    TooLarge,
    /// Reading a source member or writing output failed.
    Io(io::Error),
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMember => formatter.write_str("invalid archive member name"),
            Self::NotAnArchive => formatter.write_str("not an encrypted PMC archive"),
            Self::CannotDecrypt => {
                formatter.write_str("wrong passphrase, or the archive header is damaged")
            }
            Self::WorkFactorTooHigh => formatter.write_str("archive work factor too high"),
            Self::Corrupt => formatter.write_str("archive is damaged or truncated"),
            Self::UnexpectedEntry => formatter.write_str("archive holds an unexpected entry"),
            Self::ManifestMismatch => formatter.write_str("archive manifest does not match"),
            Self::TooLarge => formatter.write_str("archive exceeds a size limit"),
            Self::Io(error) => write!(formatter, "archive I/O failed: {error}"),
        }
    }
}

impl std::error::Error for ArchiveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_MEMBER_NAME
        && name != "."
        && name != ".."
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A writer that hashes and counts everything passing through it.
struct HashingWriter<W> {
    inner: W,
    hasher: Sha256,
    bytes: u64,
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.hasher.update(&buf[..written]);
        self.bytes += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// A reader that fails once more than `remaining` bytes have come through.
struct CappedReader<R> {
    inner: R,
    remaining: u64,
}

impl<R: Read> Read for CappedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            // At the ceiling: one more byte means the stream is over it.
            let mut probe = [0u8; 1];
            return match self.inner.read(&mut probe)? {
                0 => Ok(0),
                _ => Err(io::Error::other(CapExceeded)),
            };
        }
        // Never ask the decoder for more than the ceiling still allows.
        let allowed = usize::try_from(self.remaining)
            .unwrap_or(usize::MAX)
            .min(buf.len());
        let read = self.inner.read(&mut buf[..allowed])?;
        self.remaining -= read as u64;
        Ok(read)
    }
}

/// The typed marker a [`CappedReader`] puts inside its `io::Error`.
#[derive(Debug)]
struct CapExceeded;

impl fmt::Display for CapExceeded {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("archive plaintext exceeds its ceiling")
    }
}

impl std::error::Error for CapExceeded {}

/// Whether the ceiling marker is anywhere in this error's chain; tar and
/// zstd may wrap the reader's error before it reaches us.
fn is_cap_exceeded(error: &io::Error) -> bool {
    let mut current: Option<&(dyn std::error::Error + 'static)> =
        error.get_ref().map(|inner| inner as _);
    while let Some(inner) = current {
        if inner.is::<CapExceeded>() {
            return true;
        }
        if let Some(io_error) = inner.downcast_ref::<io::Error>() {
            if is_cap_exceeded(io_error) {
                return true;
            }
        }
        current = inner.source();
    }
    false
}

/// A reader that hashes and counts everything read through it.
struct HashingReader<R> {
    inner: R,
    hasher: Sha256,
    bytes: u64,
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.hasher.update(&buf[..read]);
        self.bytes += read as u64;
        Ok(read)
    }
}

fn plain_header(name: &str, size: u64) -> io::Result<tar::Header> {
    // Deterministic: no timestamps, owners or host permissions.
    let mut header = tar::Header::new_ustar();
    header.set_path(name)?;
    header.set_size(size);
    header.set_mode(0o600);
    header.set_mtime(0);
    header.set_uid(0);
    header.set_gid(0);
    header.set_entry_type(tar::EntryType::Regular);
    header.set_cksum();
    Ok(header)
}

/// Write one archive to `output` at the ADR's work factor. Returns its
/// manifest and container checksum. `output` holds a complete, finished age
/// file only when this returns `Ok`; publication (staging, fsync, rename) is
/// the caller's.
pub fn write_archive<W: Write>(
    output: W,
    passphrase: &Passphrase,
    input: &ManifestInput,
    members: &[ArchiveMemberSource<'_>],
) -> Result<WrittenArchive, ArchiveError> {
    write_archive_with(output, passphrase, PRODUCTION_WORK_FACTOR, input, members)
}

/// A preservation copy's container (DG3 restore-unopened amendment §1): the
/// exact Ledger files PMC could not open, kept byte for byte. Written like an
/// Operational Backup but under its own profile; [`read_archive`] refuses it
/// and [`read_preservation_archive`] refuses anything else.
pub fn write_preservation_archive<W: Write>(
    output: W,
    passphrase: &Passphrase,
    input: &ManifestInput,
    members: &[ArchiveMemberSource<'_>],
) -> Result<WrittenArchive, ArchiveError> {
    write_profiled(
        output,
        passphrase,
        PRODUCTION_WORK_FACTOR,
        PRESERVATION_PROFILE,
        input,
        members,
    )
}

fn write_archive_with<W: Write>(
    output: W,
    passphrase: &Passphrase,
    work_factor: u8,
    input: &ManifestInput,
    members: &[ArchiveMemberSource<'_>],
) -> Result<WrittenArchive, ArchiveError> {
    write_profiled(
        output,
        passphrase,
        work_factor,
        OPERATIONAL_PROFILE,
        input,
        members,
    )
}

fn write_profiled<W: Write>(
    output: W,
    passphrase: &Passphrase,
    work_factor: u8,
    profile: &'static str,
    input: &ManifestInput,
    members: &[ArchiveMemberSource<'_>],
) -> Result<WrittenArchive, ArchiveError> {
    let mut seen = HashSet::new();
    for member in members {
        if !is_plain_name(member.name) || member.name == MANIFEST_NAME || !seen.insert(member.name)
        {
            return Err(ArchiveError::InvalidMember);
        }
    }
    // Everything the reader will refuse is refused here, before any work.
    if members.len() > MAX_MEMBERS
        || [
            &input.archive_id,
            &input.created_at,
            &input.snapshot_checksum,
        ]
        .iter()
        .any(|field| field.len() > MAX_MANIFEST_FIELD_BYTES)
    {
        return Err(ArchiveError::TooLarge);
    }
    let mut total: u64 = 0;
    for member in members {
        let size = std::fs::metadata(member.path)
            .map_err(ArchiveError::Io)?
            .len();
        total = total.saturating_add(size);
    }
    if total > MAX_ARCHIVE_PLAINTEXT_BYTES {
        return Err(ArchiveError::TooLarge);
    }
    // The manifest's length does not depend on the digests' values, so its
    // size is known before any work: refuse one the reader would refuse.
    let placeholder = manifest_for(
        input,
        profile,
        members
            .iter()
            .map(|member| ManifestMember {
                name: member.name.to_owned(),
                size: u64::MAX,
                sha256: "0".repeat(64),
            })
            .collect(),
    );
    if serde_json::to_vec(&placeholder)
        .map_err(|error| ArchiveError::Io(io::Error::other(error)))?
        .len() as u64
        > MAX_MANIFEST_BYTES
    {
        return Err(ArchiveError::TooLarge);
    }

    let mut recipient = age::scrypt::Recipient::new(passphrase.secret());
    recipient.set_work_factor(work_factor);
    let encryptor = age::Encryptor::with_recipients(std::iter::once(&recipient as _))
        .map_err(|_| ArchiveError::Io(io::Error::other("age encryptor")))?;

    let mut counted = HashingWriter {
        inner: output,
        hasher: Sha256::new(),
        bytes: 0,
    };
    let encrypted = encryptor
        .wrap_output(&mut counted)
        .map_err(ArchiveError::Io)?;
    let compressed = zstd::Encoder::new(encrypted, ZSTD_LEVEL).map_err(ArchiveError::Io)?;
    let mut builder = tar::Builder::new(compressed);

    let mut manifest_members = Vec::with_capacity(members.len());
    for member in members {
        let file = File::open(member.path).map_err(ArchiveError::Io)?;
        let size = file.metadata().map_err(ArchiveError::Io)?.len();
        let mut reader = HashingReader {
            inner: BufReader::new(file),
            hasher: Sha256::new(),
            bytes: 0,
        };
        let header = plain_header(member.name, size).map_err(ArchiveError::Io)?;
        builder
            .append(&header, &mut reader)
            .map_err(ArchiveError::Io)?;
        // A source that changed size while being read is not archived.
        if reader.bytes != size {
            return Err(ArchiveError::Io(io::Error::other(
                "archive member changed while it was read",
            )));
        }
        manifest_members.push(ManifestMember {
            name: member.name.to_owned(),
            size,
            sha256: hex(&reader.hasher.finalize()),
        });
    }

    let manifest = manifest_for(input, profile, manifest_members);
    let manifest_bytes =
        serde_json::to_vec(&manifest).map_err(|error| ArchiveError::Io(io::Error::other(error)))?;
    if manifest_bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(ArchiveError::TooLarge);
    }
    let header =
        plain_header(MANIFEST_NAME, manifest_bytes.len() as u64).map_err(ArchiveError::Io)?;
    builder
        .append(&header, manifest_bytes.as_slice())
        .map_err(ArchiveError::Io)?;

    // Each layer must be finished, innermost first: an unfinished age stream
    // lacks its final authenticated chunk and is not a valid file.
    let compressed = builder.into_inner().map_err(ArchiveError::Io)?;
    let encrypted = compressed.finish().map_err(ArchiveError::Io)?;
    encrypted.finish().map_err(ArchiveError::Io)?;
    counted.flush().map_err(ArchiveError::Io)?;

    Ok(WrittenArchive {
        manifest,
        container_sha256: hex(&counted.hasher.finalize()),
        container_bytes: counted.bytes,
    })
}

fn manifest_for(
    input: &ManifestInput,
    profile: &'static str,
    members: Vec<ManifestMember>,
) -> ArchiveManifest {
    ArchiveManifest {
        format: ARCHIVE_FORMAT.to_owned(),
        manifest_format: MANIFEST_FORMAT.to_owned(),
        profile: profile.to_owned(),
        archive_id: input.archive_id.clone(),
        created_at: input.created_at.clone(),
        ledger_schema_version: input.ledger_schema_version,
        ledger_revision: input.ledger_revision,
        snapshot_checksum: input.snapshot_checksum.clone(),
        members,
    }
}

fn open_decryptor<R: Read>(
    input: R,
    passphrase: &Passphrase,
    max_work_factor: u8,
) -> Result<age::stream::StreamReader<R>, ArchiveError> {
    let decryptor = age::Decryptor::new(input).map_err(|error| match error {
        age::DecryptError::Io(_) => ArchiveError::Corrupt,
        _ => ArchiveError::NotAnArchive,
    })?;
    if !decryptor.is_scrypt() {
        return Err(ArchiveError::NotAnArchive);
    }
    let mut identity = age::scrypt::Identity::new(passphrase.secret());
    identity.set_max_work_factor(max_work_factor);
    decryptor
        .decrypt(std::iter::once(&identity as _))
        .map_err(|error| match error {
            age::DecryptError::ExcessiveWork { .. } => ArchiveError::WorkFactorTooHigh,
            age::DecryptError::DecryptionFailed
            | age::DecryptError::KeyDecryptionFailed
            | age::DecryptError::NoMatchingKeys => ArchiveError::CannotDecrypt,
            _ => ArchiveError::Corrupt,
        })
}

/// A read failure inside the decrypted, decompressed stream means damage,
/// not a local I/O problem: every byte comes from the archive. Passing a
/// ceiling is reported as such.
fn stream_error(error: io::Error) -> ArchiveError {
    if is_cap_exceeded(&error) {
        ArchiveError::TooLarge
    } else {
        ArchiveError::Corrupt
    }
}

/// Read and verify an archive, refusing a work factor above
/// [`MAX_ACCEPTED_WORK_FACTOR`]. With `extract_to`, each member is written into
/// that existing directory under its own name (never overwriting); the caller
/// must discard the directory unless this returns `Ok`.
///
/// This checks the container only. Which members an Operational Backup must
/// hold is the S7 caller's to require.
pub fn read_archive<R: Read>(
    input: R,
    passphrase: &Passphrase,
    extract_to: Option<&Path>,
) -> Result<VerifiedArchive, ArchiveError> {
    read_archive_with(input, passphrase, MAX_ACCEPTED_WORK_FACTOR, extract_to)
}

/// Read and verify a preservation copy (see [`write_preservation_archive`]),
/// as [`read_archive`] reads an Operational Backup.
pub fn read_preservation_archive<R: Read>(
    input: R,
    passphrase: &Passphrase,
    extract_to: Option<&Path>,
) -> Result<VerifiedArchive, ArchiveError> {
    read_profiled(
        input,
        passphrase,
        MAX_ACCEPTED_WORK_FACTOR,
        extract_to,
        MAX_ARCHIVE_PLAINTEXT_BYTES + MAX_MANIFEST_BYTES + TAR_OVERHEAD_BYTES,
        PRESERVATION_PROFILE,
    )
}

fn read_archive_with<R: Read>(
    input: R,
    passphrase: &Passphrase,
    max_work_factor: u8,
    extract_to: Option<&Path>,
) -> Result<VerifiedArchive, ArchiveError> {
    read_archive_capped(
        input,
        passphrase,
        max_work_factor,
        extract_to,
        MAX_ARCHIVE_PLAINTEXT_BYTES + MAX_MANIFEST_BYTES + TAR_OVERHEAD_BYTES,
    )
}

fn read_archive_capped<R: Read>(
    input: R,
    passphrase: &Passphrase,
    max_work_factor: u8,
    extract_to: Option<&Path>,
    stream_ceiling: u64,
) -> Result<VerifiedArchive, ArchiveError> {
    read_profiled(
        input,
        passphrase,
        max_work_factor,
        extract_to,
        stream_ceiling,
        OPERATIONAL_PROFILE,
    )
}

fn read_profiled<R: Read>(
    input: R,
    passphrase: &Passphrase,
    max_work_factor: u8,
    extract_to: Option<&Path>,
    stream_ceiling: u64,
    profile: &'static str,
) -> Result<VerifiedArchive, ArchiveError> {
    let decrypted = open_decryptor(
        input,
        passphrase,
        max_work_factor.min(MAX_ACCEPTED_WORK_FACTOR),
    )?;
    let mut decoder = zstd::Decoder::new(decrypted).map_err(stream_error)?;
    decoder
        .window_log_max(ZSTD_WINDOW_LOG_MAX)
        .map_err(stream_error)?;
    let capped = CappedReader {
        inner: decoder,
        remaining: stream_ceiling,
    };
    let mut archive = tar::Archive::new(capped);
    let mut declared_total: u64 = 0;

    let mut computed: Vec<ManifestMember> = Vec::new();
    let mut manifest: Option<ArchiveManifest> = None;
    {
        // Raw: tar would otherwise apply PAX and GNU extension records
        // itself; here they are entries of their own, and refused.
        let entries = archive.entries().map_err(stream_error)?.raw(true);
        for entry in entries {
            let mut entry = entry.map_err(stream_error)?;
            if manifest.is_some() {
                return Err(ArchiveError::UnexpectedEntry);
            }
            let header = entry.header();
            if header.entry_type() != tar::EntryType::Regular {
                return Err(ArchiveError::UnexpectedEntry);
            }
            let raw_name = entry.path_bytes();
            let name = std::str::from_utf8(&raw_name)
                .map_err(|_| ArchiveError::UnexpectedEntry)?
                .to_owned();
            if !is_plain_name(&name) || computed.iter().any(|member| member.name == name) {
                return Err(ArchiveError::UnexpectedEntry);
            }
            let size = entry.size();
            // Ceilings on what the entry declares, before any byte of it is
            // read or written.
            declared_total = declared_total.saturating_add(size);
            if declared_total > MAX_ARCHIVE_PLAINTEXT_BYTES + MAX_MANIFEST_BYTES
                || (name != MANIFEST_NAME && computed.len() >= MAX_MEMBERS)
            {
                return Err(ArchiveError::TooLarge);
            }

            if name == MANIFEST_NAME {
                if size > MAX_MANIFEST_BYTES {
                    return Err(ArchiveError::TooLarge);
                }
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).map_err(stream_error)?;
                manifest = Some(
                    serde_json::from_slice(&bytes).map_err(|_| ArchiveError::ManifestMismatch)?,
                );
                continue;
            }

            let mut reader = HashingReader {
                inner: &mut entry,
                hasher: Sha256::new(),
                bytes: 0,
            };
            match extract_to {
                Some(dir) => {
                    let mut file = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(dir.join(&name))
                        .map_err(ArchiveError::Io)?;
                    copy_member(&mut reader, &mut file)?;
                    file.sync_all().map_err(ArchiveError::Io)?;
                }
                None => copy_member(&mut reader, &mut io::sink())?,
            }
            if reader.bytes != size {
                return Err(ArchiveError::Corrupt);
            }
            computed.push(ManifestMember {
                name,
                size,
                sha256: hex(&reader.hasher.finalize()),
            });
        }
    }

    // age authenticates the final chunk only at the end of its stream: read
    // everything the tar reader left unread (end-of-archive padding) so a
    // truncated file cannot pass as complete.
    let mut rest = archive.into_inner();
    io::copy(&mut rest, &mut io::sink()).map_err(stream_error)?;

    let manifest = manifest.ok_or(ArchiveError::ManifestMismatch)?;
    if manifest.format != ARCHIVE_FORMAT
        || manifest.manifest_format != MANIFEST_FORMAT
        || manifest.profile != profile
        || manifest.members != computed
    {
        return Err(ArchiveError::ManifestMismatch);
    }
    Ok(VerifiedArchive { manifest })
}

/// Copies one member. A read failure is damage; a write failure is local.
fn copy_member(reader: &mut impl Read, writer: &mut impl Write) -> Result<(), ArchiveError> {
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(stream_error)?;
        if read == 0 {
            return Ok(());
        }
        writer
            .write_all(&buffer[..read])
            .map_err(ArchiveError::Io)?;
    }
}

#[cfg(test)]
mod tests;
