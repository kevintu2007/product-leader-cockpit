//! Probing a folder a person chose for backups: it must be a real, writable
//! directory, not a link, and the probe must leave nothing behind.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_platform::backup_destination::{probe_destination, DestinationProblem};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("pmc-destination-{name}-{nonce}-{sequence}"));
    fs::create_dir_all(&dir).unwrap_or_else(|error| panic!("scratch dir failed: {error}"));
    fs::canonicalize(&dir).unwrap_or_else(|error| panic!("canonicalize failed: {error}"))
}

#[test]
fn a_writable_folder_is_accepted_and_left_as_it_was() {
    let dir = scratch("writable");
    let accepted = probe_destination(dir.clone())
        .unwrap_or_else(|problem| panic!("probe refused a writable folder: {problem:?}"));
    let proven = accepted
        .revalidate()
        .unwrap_or_else(|error| panic!("revalidate failed: {error}"));
    assert_eq!(proven.as_path(), dir.as_path());
    let left = fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("read_dir failed: {error}"))
        .count();
    assert_eq!(left, 0, "the probe must leave nothing behind");
}

#[test]
fn a_missing_folder_is_refused() {
    let dir = scratch("missing").join("not-there");
    assert!(matches!(
        probe_destination(dir),
        Err(DestinationProblem::NotAFolder)
    ));
}

#[test]
fn a_file_is_refused() {
    let dir = scratch("file");
    let file = dir.join("a-file.txt");
    fs::write(&file, b"x").unwrap_or_else(|error| panic!("write failed: {error}"));
    assert!(matches!(
        probe_destination(file),
        Err(DestinationProblem::NotAFolder)
    ));
}

#[test]
fn a_relative_path_is_refused() {
    assert!(matches!(
        probe_destination(PathBuf::from("relative\\folder")),
        Err(DestinationProblem::NotAFolder)
    ));
}

#[cfg(unix)]
#[test]
fn a_read_only_folder_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch("read-only");
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o500))
        .unwrap_or_else(|error| panic!("chmod failed: {error}"));
    assert!(matches!(
        probe_destination(dir),
        Err(DestinationProblem::NotWritable)
    ));
}

#[test]
fn an_existing_file_in_the_folder_is_never_touched() {
    let dir = scratch("existing");
    let keep = dir.join(".pmc-destination-probe-keep");
    fs::write(&keep, b"someone else's").unwrap_or_else(|error| panic!("write failed: {error}"));
    probe_destination(dir.clone()).unwrap_or_else(|problem| panic!("refused: {problem:?}"));
    assert_eq!(
        fs::read(&keep).unwrap_or_default(),
        b"someone else's",
        "a file the probe did not create must survive"
    );
    let left = fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("read_dir failed: {error}"))
        .count();
    assert_eq!(left, 1);
}
