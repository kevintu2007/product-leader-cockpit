//! The folder Operational Backups are published to (ADR 0010, ADR 0011).
//!
//! A person chooses it through the host's native folder picker; the path
//! never reaches the webview. Before it is stored, it is proven to be a real
//! directory (canonical, absolute, no link or reparse point anywhere on the
//! way) that this account can create and remove a file in. It is stored as a
//! [`DeferredDirectoryPath`] and proven again every time it is used.

use std::collections::hash_map::RandomState;
use std::fs::{self, OpenOptions};
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Write};
use std::path::PathBuf;

use crate::settings::{CanonicalDirectoryPath, DeferredDirectoryPath};

const PROBE_ATTEMPTS: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DestinationProblem {
    /// Missing, not a directory, not absolute, or reached through a link.
    NotAFolder,
    /// A directory, but a file cannot be created and removed in it.
    NotWritable,
}

/// A name no other file in the folder is expected to have: 64 bits from the
/// process's randomly seeded hasher.
fn probe_name() -> String {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos()),
    );
    format!(".pmc-destination-probe-{:016x}", hasher.finish())
}

/// Prove `candidate` usable for backups and return it in the form settings
/// store. Only a probe file this call created is ever removed; an existing
/// file of the same name is left alone and another name is tried.
pub fn probe_destination(candidate: PathBuf) -> Result<DeferredDirectoryPath, DestinationProblem> {
    let canonical = fs::canonicalize(&candidate).map_err(|_| DestinationProblem::NotAFolder)?;
    let directory =
        CanonicalDirectoryPath::new(canonical).map_err(|_| DestinationProblem::NotAFolder)?;
    for _ in 0..PROBE_ATTEMPTS {
        let probe = directory.as_path().join(probe_name());
        let created = OpenOptions::new().write(true).create_new(true).open(&probe);
        let mut file = match created {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(DestinationProblem::NotWritable),
        };
        let written = file.write_all(b"pmc").and_then(|()| file.sync_all());
        drop(file);
        // Ours: created by this call, so removing it deletes nothing else.
        let removed = fs::remove_file(&probe);
        return match (written, removed) {
            (Ok(()), Ok(())) => Ok(DeferredDirectoryPath::from(directory)),
            _ => Err(DestinationProblem::NotWritable),
        };
    }
    Err(DestinationProblem::NotWritable)
}
