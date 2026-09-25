//! Preservation copies (DG3 restore-unopened amendment §1; cut B2): the exact
//! Ledger files, encrypted into the backup folder, read back and matched byte
//! for byte — and never taken for an Operational Backup, or the reverse.

use std::fs;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_platform::backup_archive::{
    read_archive, read_preservation_archive, write_archive, ArchiveMemberSource, ManifestInput,
    Passphrase,
};
use pmc_platform::backup_registry::{orphan_archives, BackupRegistry};
use pmc_platform::preservation::{
    bind_ledger_files, is_preservation_file_name, preservation_copy_matches, preserve_ledger_files,
    PreservationError,
};
use pmc_platform::restore_control::LedgerFileMember;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let directory =
        std::env::temp_dir().join(format!("pmc-preservation-{name}-{nonce}-{sequence}"));
    fs::create_dir_all(&directory).unwrap_or_else(|error| panic!("{error}"));
    directory
}

fn passphrase() -> Passphrase {
    Passphrase::new("correct horse battery staple river".to_owned())
}

/// A "Ledger" that does not open: arbitrary bytes, and sidecars when asked.
fn broken_ledger(directory: &Path, sidecars: bool) -> PathBuf {
    let ledger = directory.join("product-ledger.sqlite3");
    fs::write(&ledger, b"not a sqlite file \x00\x01\x02").unwrap_or_else(|error| panic!("{error}"));
    if sidecars {
        fs::write(directory.join("product-ledger.sqlite3-wal"), b"wal frames")
            .unwrap_or_else(|error| panic!("{error}"));
        fs::write(
            directory.join("product-ledger.sqlite3-shm"),
            vec![7u8; 32_768],
        )
        .unwrap_or_else(|error| panic!("{error}"));
    }
    ledger
}

const NAME: &str = "pmc-preservation-20260923T010203000Z-p1p1p1p1-v1.tar.zst.age";

#[test]
fn the_ledger_file_alone_is_kept_and_matches_byte_for_byte() {
    let work = scratch("main-only");
    let folder = scratch("main-only-folder");
    let ledger = broken_ledger(&work, false);
    let copy = preserve_ledger_files(
        &ledger,
        &folder,
        &passphrase(),
        "p1p1p1p1",
        "2026-09-23T01:02:03.000Z",
        NAME,
    )
    .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(copy.files.members().len(), 1);
    assert_eq!(
        copy.files,
        bind_ledger_files(&ledger).unwrap_or_else(|e| panic!("{e}"))
    );
    assert!(preservation_copy_matches(&folder, &copy, &passphrase()));
    // Only the finished copy is left in the folder.
    let names: Vec<String> = fs::read_dir(&folder)
        .unwrap_or_else(|error| panic!("{error}"))
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec![NAME.to_owned()]);
}

#[test]
fn the_ledger_and_both_sidecars_are_kept_exactly() {
    let work = scratch("sidecars");
    let folder = scratch("sidecars-folder");
    let ledger = broken_ledger(&work, true);
    let copy = preserve_ledger_files(
        &ledger,
        &folder,
        &passphrase(),
        "p2p2p2p2",
        "2026-09-23T01:02:03.000Z",
        NAME,
    )
    .unwrap_or_else(|error| panic!("{error}"));
    let members: Vec<LedgerFileMember> = copy
        .files
        .members()
        .iter()
        .map(|member| member.member)
        .collect();
    assert_eq!(
        members,
        [
            LedgerFileMember::Ledger,
            LedgerFileMember::Wal,
            LedgerFileMember::Shm
        ]
    );
    // Extracted, every member is the file it came from.
    let extract = scratch("sidecars-extract");
    let file = fs::File::open(folder.join(NAME)).unwrap_or_else(|error| panic!("{error}"));
    read_preservation_archive(BufReader::new(file), &passphrase(), Some(&extract))
        .unwrap_or_else(|error| panic!("{error}"));
    for (member, source) in [
        ("ledger", "product-ledger.sqlite3"),
        ("ledger-wal", "product-ledger.sqlite3-wal"),
        ("ledger-shm", "product-ledger.sqlite3-shm"),
    ] {
        assert_eq!(
            fs::read(extract.join(member)).unwrap_or_else(|error| panic!("{error}")),
            fs::read(work.join(source)).unwrap_or_else(|error| panic!("{error}")),
            "{member}"
        );
    }
}

#[test]
fn a_damaged_copy_or_the_wrong_passphrase_does_not_match() {
    let work = scratch("damaged");
    let folder = scratch("damaged-folder");
    let ledger = broken_ledger(&work, true);
    let copy = preserve_ledger_files(
        &ledger,
        &folder,
        &passphrase(),
        "p3p3p3p3",
        "2026-09-23T01:02:03.000Z",
        NAME,
    )
    .unwrap_or_else(|error| panic!("{error}"));
    assert!(!preservation_copy_matches(
        &folder,
        &copy,
        &Passphrase::new("another passphrase entirely".to_owned())
    ));
    let path = folder.join(NAME);
    let mut bytes = fs::read(&path).unwrap_or_else(|error| panic!("{error}"));
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0x01;
    fs::write(&path, bytes).unwrap_or_else(|error| panic!("{error}"));
    assert!(!preservation_copy_matches(&folder, &copy, &passphrase()));
}

#[test]
fn nothing_is_left_behind_or_overwritten_when_a_copy_cannot_be_made() {
    let work = scratch("refused");
    let folder = scratch("refused-folder");
    // No Ledger file at all.
    assert!(matches!(
        preserve_ledger_files(
            &work.join("product-ledger.sqlite3"),
            &folder,
            &passphrase(),
            "p4p4p4p4",
            "2026-09-23T01:02:03.000Z",
            NAME,
        ),
        Err(PreservationError::Unreadable(_))
    ));
    // Not a preservation copy's name.
    let ledger = broken_ledger(&work, false);
    assert!(matches!(
        preserve_ledger_files(
            &ledger,
            &folder,
            &passphrase(),
            "p4p4p4p4",
            "2026-09-23T01:02:03.000Z",
            "pmc-operational-20260923-p4-v1.tar.zst.age",
        ),
        Err(PreservationError::InvalidName)
    ));
    // A file already by that name is never overwritten.
    fs::write(folder.join(NAME), b"someone else's").unwrap_or_else(|error| panic!("{error}"));
    assert!(matches!(
        preserve_ledger_files(
            &ledger,
            &folder,
            &passphrase(),
            "p4p4p4p4",
            "2026-09-23T01:02:03.000Z",
            NAME,
        ),
        Err(PreservationError::InvalidName)
    ));
    assert_eq!(
        fs::read(folder.join(NAME)).unwrap_or_else(|error| panic!("{error}")),
        b"someone else's"
    );
    let names: Vec<String> = fs::read_dir(&folder)
        .unwrap_or_else(|error| panic!("{error}"))
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec![NAME.to_owned()]);
}

#[test]
fn a_preservation_copy_is_never_an_operational_backup_and_the_reverse() {
    let work = scratch("apart");
    let folder = scratch("apart-folder");
    let ledger = broken_ledger(&work, false);
    preserve_ledger_files(
        &ledger,
        &folder,
        &passphrase(),
        "p5p5p5p5",
        "2026-09-23T01:02:03.000Z",
        NAME,
    )
    .unwrap_or_else(|error| panic!("{error}"));
    assert!(is_preservation_file_name(NAME));
    // The registry's orphan scan does not see it.
    assert!(orphan_archives(&folder, &BackupRegistry::default()).is_empty());
    // An Operational Backup reader refuses it, even under the right passphrase.
    let file = fs::File::open(folder.join(NAME)).unwrap_or_else(|error| panic!("{error}"));
    assert!(read_archive(BufReader::new(file), &passphrase(), None).is_err());

    // And an Operational Backup is not read as a preservation copy.
    let operational = folder.join("operational.age");
    let output = fs::File::create(&operational).unwrap_or_else(|error| panic!("{error}"));
    write_archive(
        &output,
        &passphrase(),
        &ManifestInput {
            archive_id: "o1o1o1o1".to_owned(),
            created_at: "2026-09-23T01:02:03.000Z".to_owned(),
            ledger_schema_version: 48,
            ledger_revision: 1,
            snapshot_checksum: "0".repeat(64),
        },
        &[ArchiveMemberSource {
            name: "ledger",
            path: &ledger,
        }],
    )
    .unwrap_or_else(|error| panic!("{error}"));
    drop(output);
    let file = fs::File::open(&operational).unwrap_or_else(|error| panic!("{error}"));
    assert!(read_preservation_archive(BufReader::new(file), &passphrase(), None).is_err());
}
