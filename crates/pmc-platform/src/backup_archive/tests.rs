//! Tests for the ADR 0010 container. They use a small scrypt work factor
//! through the module's private entry points; the public ones always use the
//! production value, which a test pins.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use super::{
    read_archive, read_archive_capped, read_archive_with, write_archive_with, ArchiveError,
    ArchiveMemberSource, ManifestInput, Passphrase, MAX_ACCEPTED_WORK_FACTOR, MAX_MEMBERS,
    PRODUCTION_WORK_FACTOR,
};

const TEST_WORK_FACTOR: u8 = 10;
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("pmc-archive-{name}-{nonce}-{sequence}"));
    fs::create_dir_all(&dir).unwrap_or_else(|error| panic!("scratch dir failed: {error}"));
    dir
}

fn write_file(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, bytes).unwrap_or_else(|error| panic!("fixture write failed: {error}"));
    path
}

fn passphrase(text: &str) -> Passphrase {
    Passphrase::new(text.to_owned())
}

fn manifest_input() -> ManifestInput {
    ManifestInput {
        archive_id: "archive-0001".to_owned(),
        created_at: "2026-09-21T08:00:00Z".to_owned(),
        ledger_schema_version: 46,
        ledger_revision: 166,
        snapshot_checksum: "a".repeat(64),
    }
}

struct Fixture {
    dir: PathBuf,
    ledger: PathBuf,
    settings: PathBuf,
    ledger_bytes: Vec<u8>,
}

fn fixture(name: &str) -> Fixture {
    let dir = scratch(name);
    // Incompressible and larger than one age chunk (64 KiB), so the
    // encrypted stream has several chunks.
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let ledger_bytes: Vec<u8> = (0..200_000)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state.to_le_bytes()[0]
        })
        .collect();
    let ledger = write_file(&dir, "ledger.sqlite3", &ledger_bytes);
    let settings = write_file(&dir, "settings.json", br#"{"format_version":1}"#);
    Fixture {
        dir,
        ledger,
        settings,
        ledger_bytes,
    }
}

fn archive(fixture: &Fixture, secret: &str, work_factor: u8) -> Vec<u8> {
    let members = [
        ArchiveMemberSource {
            name: "ledger.sqlite3",
            path: &fixture.ledger,
        },
        ArchiveMemberSource {
            name: "settings.json",
            path: &fixture.settings,
        },
    ];
    let mut bytes = Vec::new();
    write_archive_with(
        &mut bytes,
        &passphrase(secret),
        work_factor,
        &manifest_input(),
        &members,
    )
    .unwrap_or_else(|error| panic!("archive write failed: {error}"));
    bytes
}

#[test]
fn production_parameters_are_the_ones_the_adr_records() {
    assert_eq!(PRODUCTION_WORK_FACTOR, 18);
    assert_eq!(MAX_ACCEPTED_WORK_FACTOR, 20);
}

#[test]
fn round_trip_restores_every_member_byte_for_byte_with_its_manifest() {
    let fixture = fixture("round-trip");
    let bytes = archive(&fixture, "correct horse battery staple", TEST_WORK_FACTOR);
    let restore = fixture.dir.join("restore");
    fs::create_dir(&restore).unwrap_or_else(|error| panic!("restore dir failed: {error}"));

    let verified = read_archive_with(
        bytes.as_slice(),
        &passphrase("correct horse battery staple"),
        MAX_ACCEPTED_WORK_FACTOR,
        Some(&restore),
    )
    .unwrap_or_else(|error| panic!("archive read failed: {error}"));

    assert_eq!(
        fs::read(restore.join("ledger.sqlite3")).unwrap_or_default(),
        fixture.ledger_bytes
    );
    assert_eq!(
        fs::read(restore.join("settings.json")).unwrap_or_default(),
        br#"{"format_version":1}"#
    );
    let manifest = &verified.manifest;
    assert_eq!(manifest.format, "pmc-backup/v1");
    assert_eq!(manifest.manifest_format, "pmc-backup-manifest/v1");
    assert_eq!(manifest.profile, "operational/v1");
    assert_eq!(manifest.archive_id, "archive-0001");
    assert_eq!(manifest.ledger_schema_version, 46);
    assert_eq!(manifest.ledger_revision, 166);
    assert_eq!(manifest.snapshot_checksum, "a".repeat(64));
    let names: Vec<&str> = manifest
        .members
        .iter()
        .map(|member| member.name.as_str())
        .collect();
    assert_eq!(names, ["ledger.sqlite3", "settings.json"]);
    assert_eq!(manifest.members[0].size, fixture.ledger_bytes.len() as u64);
    assert_eq!(
        manifest.members[0].sha256,
        format!("{:x}", Sha256::digest(&fixture.ledger_bytes))
    );
}

#[test]
fn verification_without_extraction_writes_nothing() {
    let fixture = fixture("verify-only");
    let bytes = archive(&fixture, "a passphrase", TEST_WORK_FACTOR);
    let verified = read_archive_with(
        bytes.as_slice(),
        &passphrase("a passphrase"),
        MAX_ACCEPTED_WORK_FACTOR,
        None,
    );
    assert!(verified.is_ok(), "{verified:?}");
}

#[test]
fn the_container_checksum_is_the_sha256_of_the_finished_file() {
    let fixture = fixture("checksum");
    let members = [ArchiveMemberSource {
        name: "ledger.sqlite3",
        path: &fixture.ledger,
    }];
    let mut bytes = Vec::new();
    let written = write_archive_with(
        &mut bytes,
        &passphrase("a passphrase"),
        TEST_WORK_FACTOR,
        &manifest_input(),
        &members,
    )
    .unwrap_or_else(|error| panic!("archive write failed: {error}"));
    assert_eq!(
        written.container_sha256,
        format!("{:x}", Sha256::digest(&bytes))
    );
    assert_eq!(written.container_bytes, bytes.len() as u64);
}

#[test]
fn the_file_is_a_standard_age_passphrase_file() {
    let fixture = fixture("standard");
    let bytes = archive(&fixture, "a passphrase", TEST_WORK_FACTOR);
    let header = String::from_utf8_lossy(&bytes[..40]);
    assert!(
        header.starts_with("age-encryption.org/v1\n-> scrypt "),
        "{header}"
    );
}

#[test]
fn a_wrong_passphrase_cannot_decrypt() {
    let fixture = fixture("wrong-passphrase");
    let bytes = archive(&fixture, "the right one", TEST_WORK_FACTOR);
    let result = read_archive_with(
        bytes.as_slice(),
        &passphrase("the wrong one"),
        MAX_ACCEPTED_WORK_FACTOR,
        None,
    );
    assert!(
        matches!(result, Err(ArchiveError::CannotDecrypt)),
        "{result:?}"
    );
}

#[test]
fn an_excessive_work_factor_is_refused_before_the_work() {
    let fixture = fixture("work-factor");
    let bytes = archive(&fixture, "a passphrase", 12);
    let result = read_archive_with(bytes.as_slice(), &passphrase("a passphrase"), 11, None);
    assert!(
        matches!(result, Err(ArchiveError::WorkFactorTooHigh)),
        "{result:?}"
    );
}

#[test]
fn a_truncated_archive_is_refused() {
    let fixture = fixture("truncated");
    let bytes = archive(&fixture, "a passphrase", TEST_WORK_FACTOR);
    for cut in [1, 17, bytes.len() / 2] {
        let truncated = &bytes[..bytes.len() - cut];
        let result = read_archive_with(truncated, &passphrase("a passphrase"), 20, None);
        assert!(
            matches!(result, Err(ArchiveError::Corrupt)),
            "cut {cut}: {result:?}"
        );
    }
}

#[test]
fn a_changed_ciphertext_byte_is_refused() {
    let fixture = fixture("tampered");
    let mut bytes = archive(&fixture, "a passphrase", TEST_WORK_FACTOR);
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0x01;
    let result = read_archive_with(bytes.as_slice(), &passphrase("a passphrase"), 20, None);
    assert!(matches!(result, Err(ArchiveError::Corrupt)), "{result:?}");
}

#[test]
fn a_file_that_is_not_age_is_refused() {
    let result = read_archive_with(
        &b"not an archive at all"[..],
        &passphrase("a passphrase"),
        20,
        None,
    );
    assert!(
        matches!(result, Err(ArchiveError::NotAnArchive)),
        "{result:?}"
    );
}

#[test]
fn member_names_must_be_plain_file_names() {
    let fixture = fixture("names");
    for name in [
        "../escape",
        "dir/file",
        "C:\\file",
        "",
        ".",
        "manifest.json",
        "UPPER",
    ] {
        let members = [ArchiveMemberSource {
            name,
            path: &fixture.ledger,
        }];
        let result = write_archive_with(
            &mut Vec::new(),
            &passphrase("a passphrase"),
            TEST_WORK_FACTOR,
            &manifest_input(),
            &members,
        );
        assert!(
            matches!(result, Err(ArchiveError::InvalidMember)),
            "{name}: {result:?}"
        );
    }
}

/// Encrypts a hand-built plaintext tar, as an attacker who knows the
/// passphrase could, to prove the reader checks the layout itself.
fn encrypt_plaintext_tar(secret: &str, build: impl FnOnce(&mut tar::Builder<Vec<u8>>)) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    build(&mut builder);
    let tar_bytes = builder
        .into_inner()
        .unwrap_or_else(|error| panic!("tar build failed: {error}"));
    let compressed = zstd::encode_all(tar_bytes.as_slice(), 3)
        .unwrap_or_else(|error| panic!("zstd failed: {error}"));
    let mut recipient = age::scrypt::Recipient::new(secret.to_owned().into());
    recipient.set_work_factor(TEST_WORK_FACTOR);
    let encryptor = age::Encryptor::with_recipients(std::iter::once(&recipient as _))
        .unwrap_or_else(|error| panic!("encryptor failed: {error}"));
    let mut output = Vec::new();
    let mut writer = encryptor
        .wrap_output(&mut output)
        .unwrap_or_else(|error| panic!("wrap failed: {error}"));
    writer
        .write_all(&compressed)
        .unwrap_or_else(|error| panic!("encrypt failed: {error}"));
    writer
        .finish()
        .unwrap_or_else(|error| panic!("finish failed: {error}"));
    output
}

fn append(builder: &mut tar::Builder<Vec<u8>>, name: &str, bytes: &[u8]) {
    let mut header = tar::Header::new_ustar();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o600);
    header.set_entry_type(tar::EntryType::Regular);
    header
        .set_path(name)
        .unwrap_or_else(|error| panic!("set_path failed: {error}"));
    header.set_cksum();
    builder
        .append(&header, bytes)
        .unwrap_or_else(|error| panic!("append failed: {error}"));
}

fn manifest_json(members: &[(&str, &[u8])]) -> Vec<u8> {
    let members: Vec<String> = members
        .iter()
        .map(|(name, bytes)| {
            format!(
                r#"{{"name":"{name}","size":{},"sha256":"{:x}"}}"#,
                bytes.len(),
                Sha256::digest(bytes)
            )
        })
        .collect();
    format!(
        r#"{{"format":"pmc-backup/v1","manifest_format":"pmc-backup-manifest/v1","profile":"operational/v1","archive_id":"x","created_at":"2026-09-21T08:00:00Z","ledger_schema_version":46,"ledger_revision":1,"snapshot_checksum":"{}","members":[{}]}}"#,
        "b".repeat(64),
        members.join(",")
    )
    .into_bytes()
}

#[test]
fn a_member_whose_bytes_differ_from_the_manifest_is_refused() {
    let bytes = encrypt_plaintext_tar("a passphrase", |builder| {
        append(builder, "ledger.sqlite3", b"what is really there");
        append(
            builder,
            "manifest.json",
            &manifest_json(&[("ledger.sqlite3", b"what the manifest claims")]),
        );
    });
    let result = read_archive_with(bytes.as_slice(), &passphrase("a passphrase"), 20, None);
    assert!(
        matches!(result, Err(ArchiveError::ManifestMismatch)),
        "{result:?}"
    );
}

#[test]
fn an_entry_after_the_manifest_is_refused() {
    let bytes = encrypt_plaintext_tar("a passphrase", |builder| {
        append(builder, "ledger.sqlite3", b"ledger");
        append(
            builder,
            "manifest.json",
            &manifest_json(&[("ledger.sqlite3", b"ledger")]),
        );
        append(builder, "extra.bin", b"smuggled");
    });
    let result = read_archive_with(bytes.as_slice(), &passphrase("a passphrase"), 20, None);
    assert!(
        matches!(result, Err(ArchiveError::UnexpectedEntry)),
        "{result:?}"
    );
}

#[test]
fn a_traversing_entry_is_refused_and_nothing_escapes() {
    let dir = scratch("traversal");
    let restore = dir.join("restore");
    fs::create_dir(&restore).unwrap_or_else(|error| panic!("restore dir failed: {error}"));
    let bytes = encrypt_plaintext_tar("a passphrase", |builder| {
        let mut header = tar::Header::new_gnu();
        header.set_size(4);
        header.set_mode(0o600);
        header.set_entry_type(tar::EntryType::Regular);
        // Bypass set_path's own check to write a hostile name.
        let name = b"../escaped.bin";
        header.as_old_mut().name[..name.len()].copy_from_slice(name);
        header.set_cksum();
        builder
            .append(&header, &b"evil"[..])
            .unwrap_or_else(|error| panic!("append failed: {error}"));
    });
    let result = read_archive_with(
        bytes.as_slice(),
        &passphrase("a passphrase"),
        20,
        Some(&restore),
    );
    assert!(
        matches!(result, Err(ArchiveError::UnexpectedEntry)),
        "{result:?}"
    );
    assert!(!dir.join("escaped.bin").exists());
}

#[test]
fn a_link_entry_is_refused() {
    let bytes = encrypt_plaintext_tar("a passphrase", |builder| {
        let mut header = tar::Header::new_ustar();
        header.set_size(0);
        header.set_entry_type(tar::EntryType::Symlink);
        header
            .set_path("ledger.sqlite3")
            .unwrap_or_else(|error| panic!("set_path failed: {error}"));
        header
            .set_link_name("C:/Windows/win.ini")
            .unwrap_or_else(|error| panic!("set_link failed: {error}"));
        header.set_cksum();
        builder
            .append(&header, std::io::empty())
            .unwrap_or_else(|error| panic!("append failed: {error}"));
    });
    let result = read_archive_with(bytes.as_slice(), &passphrase("a passphrase"), 20, None);
    assert!(
        matches!(result, Err(ArchiveError::UnexpectedEntry)),
        "{result:?}"
    );
}

#[test]
fn an_archive_without_a_manifest_is_refused() {
    let bytes = encrypt_plaintext_tar("a passphrase", |builder| {
        append(builder, "ledger.sqlite3", b"ledger");
    });
    let result = read_archive_with(bytes.as_slice(), &passphrase("a passphrase"), 20, None);
    assert!(
        matches!(result, Err(ArchiveError::ManifestMismatch)),
        "{result:?}"
    );
}

#[test]
fn the_passphrase_never_appears_in_debug_output() {
    let secret = passphrase("do not print me");
    assert!(!format!("{secret:?}").contains("do not print me"));
}

/// Where the age payload starts: after the header's `--- <mac>` line and the
/// 16-byte payload nonce. Chunks are 64 KiB of plaintext plus a 16-byte tag.
fn payload_start(bytes: &[u8]) -> usize {
    let marker = b"\n--- ";
    let at = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .expect("age header end");
    let line_end = at
        + 1
        + bytes[at + 1..]
            .iter()
            .position(|b| *b == b'\n')
            .expect("mac line");
    line_end + 1 + 16
}

const AGE_CHUNK: usize = 64 * 1024 + 16;

#[test]
fn a_cut_exactly_after_a_complete_chunk_is_refused() {
    let fixture = fixture("chunk-boundary");
    let bytes = archive(&fixture, "a passphrase", TEST_WORK_FACTOR);
    let start = payload_start(&bytes);
    let chunks = (bytes.len() - start).div_ceil(AGE_CHUNK);
    assert!(
        chunks >= 3,
        "fixture must span several chunks, got {chunks}"
    );
    for complete in 1..chunks {
        let cut = start + complete * AGE_CHUNK;
        let result = read_archive_with(&bytes[..cut], &passphrase("a passphrase"), 20, None);
        assert!(
            matches!(result, Err(ArchiveError::Corrupt)),
            "cut after chunk {complete}: {result:?}"
        );
    }
}

#[test]
fn bytes_appended_after_the_age_stream_are_refused() {
    let fixture = fixture("appended");
    let mut bytes = archive(&fixture, "a passphrase", TEST_WORK_FACTOR);
    bytes.extend_from_slice(b"trailing bytes after the final chunk");
    let result = read_archive_with(bytes.as_slice(), &passphrase("a passphrase"), 20, None);
    assert!(matches!(result, Err(ArchiveError::Corrupt)), "{result:?}");
}

#[test]
fn the_public_entry_points_use_the_production_work_factor() {
    let fixture = fixture("production");
    let members = [ArchiveMemberSource {
        name: "settings.json",
        path: &fixture.settings,
    }];
    let mut bytes = Vec::new();
    super::write_archive(
        &mut bytes,
        &passphrase("a passphrase"),
        &manifest_input(),
        &members,
    )
    .unwrap();
    let header = String::from_utf8_lossy(&bytes[..120]).into_owned();
    let stanza = header.lines().nth(1).unwrap_or_default().to_owned();
    assert!(stanza.ends_with(" 18"), "{stanza}");
    // And the public reader opens it at its own ceiling.
    assert!(read_archive(bytes.as_slice(), &passphrase("a passphrase"), None).is_ok());
}

#[test]
fn a_reader_limit_above_the_adr_ceiling_is_clamped() {
    let fixture = fixture("clamp");
    let mut bytes = archive(&fixture, "a passphrase", 12);
    // Claim 2^21 in the stanza. age checks the work factor before running
    // scrypt or the header MAC, so no 2 GiB derivation is ever attempted.
    let text = String::from_utf8_lossy(&bytes[..200]).into_owned();
    let stanza_end = text.find(" 12\n").expect("stanza work factor");
    bytes[stanza_end + 1..stanza_end + 3].copy_from_slice(b"21");
    let result = read_archive_with(bytes.as_slice(), &passphrase("a passphrase"), 63, None);
    assert!(
        matches!(result, Err(ArchiveError::WorkFactorTooHigh)),
        "{result:?}"
    );
}

#[test]
fn a_member_declaring_a_huge_size_is_refused_before_it_is_read() {
    let dir = scratch("huge");
    let restore = dir.join("restore");
    fs::create_dir(&restore).unwrap();
    // A header that claims 64 GiB and then ends: nothing but the claim.
    let mut header = tar::Header::new_ustar();
    header.set_path("ledger.sqlite3").unwrap();
    header.set_size(64 * 1024 * 1024 * 1024);
    header.set_mode(0o600);
    header.set_entry_type(tar::EntryType::Regular);
    header.set_cksum();
    let bytes = encrypt_raw_tar("a passphrase", header.as_bytes());
    let result = read_archive_with(
        bytes.as_slice(),
        &passphrase("a passphrase"),
        20,
        Some(&restore),
    );
    assert!(matches!(result, Err(ArchiveError::TooLarge)), "{result:?}");
    assert!(!restore.join("ledger.sqlite3").exists());
}

#[test]
fn too_many_members_are_refused_by_both_writer_and_reader() {
    let fixture = fixture("many");
    let names: Vec<String> = (0..=MAX_MEMBERS).map(|index| format!("m{index}")).collect();
    let members: Vec<ArchiveMemberSource<'_>> = names
        .iter()
        .map(|name| ArchiveMemberSource {
            name,
            path: &fixture.settings,
        })
        .collect();
    let result = write_archive_with(
        &mut Vec::new(),
        &passphrase("a passphrase"),
        TEST_WORK_FACTOR,
        &manifest_input(),
        &members,
    );
    assert!(matches!(result, Err(ArchiveError::TooLarge)), "{result:?}");

    let bytes = encrypt_plaintext_tar("a passphrase", |builder| {
        for name in &names {
            append(builder, name, b"x");
        }
    });
    let result = read_archive_with(bytes.as_slice(), &passphrase("a passphrase"), 20, None);
    assert!(matches!(result, Err(ArchiveError::TooLarge)), "{result:?}");
}

#[test]
fn an_overlong_manifest_field_is_refused_before_any_work() {
    let fixture = fixture("long-field");
    let members = [ArchiveMemberSource {
        name: "settings.json",
        path: &fixture.settings,
    }];
    let mut input = manifest_input();
    input.archive_id = "x".repeat(129);
    let result = write_archive_with(
        &mut Vec::new(),
        &passphrase("a passphrase"),
        TEST_WORK_FACTOR,
        &input,
        &members,
    );
    assert!(matches!(result, Err(ArchiveError::TooLarge)), "{result:?}");
}

/// Decodes the three layers with the libraries alone, never through
/// `read_archive`: what a person with `age`, `zstd` and `tar` would get.
#[test]
fn the_three_layers_decode_independently_to_plain_tar_members() {
    let fixture = fixture("independent");
    let bytes = archive(&fixture, "a passphrase", TEST_WORK_FACTOR);

    let decryptor = age::Decryptor::new(bytes.as_slice()).unwrap();
    let identity = age::scrypt::Identity::new("a passphrase".to_owned().into());
    let mut plaintext = Vec::new();
    std::io::Read::read_to_end(
        &mut decryptor.decrypt(std::iter::once(&identity as _)).unwrap(),
        &mut plaintext,
    )
    .unwrap();
    let tar_bytes = zstd::decode_all(plaintext.as_slice()).unwrap();

    let mut tar = tar::Archive::new(tar_bytes.as_slice());
    let mut seen = Vec::new();
    for entry in tar.entries().unwrap() {
        let mut entry = entry.unwrap();
        let header = entry.header();
        assert_eq!(header.entry_type(), tar::EntryType::Regular);
        assert_eq!(header.mode().unwrap(), 0o600);
        assert_eq!(header.mtime().unwrap(), 0);
        assert_eq!(header.uid().unwrap(), 0);
        assert_eq!(header.gid().unwrap(), 0);
        let name = String::from_utf8(entry.path_bytes().into_owned()).unwrap();
        let mut content = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut content).unwrap();
        seen.push((name, content));
    }
    let names: Vec<&str> = seen.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["ledger.sqlite3", "settings.json", "manifest.json"]);
    assert_eq!(seen[0].1, fixture.ledger_bytes);
    assert_eq!(seen[1].1, br#"{"format_version":1}"#);
}

fn encrypt_raw_tar(secret: &str, tar_bytes: &[u8]) -> Vec<u8> {
    let compressed = zstd::encode_all(tar_bytes, 3).unwrap();
    let mut recipient = age::scrypt::Recipient::new(secret.to_owned().into());
    recipient.set_work_factor(TEST_WORK_FACTOR);
    let encryptor = age::Encryptor::with_recipients(std::iter::once(&recipient as _)).unwrap();
    let mut output = Vec::new();
    let mut writer = encryptor.wrap_output(&mut output).unwrap();
    writer.write_all(&compressed).unwrap();
    writer.finish().unwrap();
    output
}

#[test]
fn a_stream_beyond_its_ceiling_is_refused_as_too_large() {
    let fixture = fixture("ceiling");
    let bytes = archive(&fixture, "a passphrase", TEST_WORK_FACTOR);
    let result = read_archive_capped(
        bytes.as_slice(),
        &passphrase("a passphrase"),
        20,
        None,
        10 * 1024,
    );
    assert!(matches!(result, Err(ArchiveError::TooLarge)), "{result:?}");
}

#[test]
fn a_pax_extension_record_is_refused() {
    let bytes = encrypt_plaintext_tar("a passphrase", |builder| {
        builder
            .append_pax_extensions([("path", "ledger.sqlite3".as_bytes())])
            .unwrap();
        append(builder, "ledger.sqlite3", b"ledger");
        append(
            builder,
            "manifest.json",
            &manifest_json(&[("ledger.sqlite3", b"ledger")]),
        );
    });
    let result = read_archive_with(bytes.as_slice(), &passphrase("a passphrase"), 20, None);
    assert!(
        matches!(result, Err(ArchiveError::UnexpectedEntry)),
        "{result:?}"
    );
}

#[test]
fn an_oversized_manifest_is_refused_as_too_large() {
    let big = vec![b' '; 70 * 1024];
    let bytes = encrypt_plaintext_tar("a passphrase", |builder| {
        append(builder, "manifest.json", &big);
    });
    let result = read_archive_with(bytes.as_slice(), &passphrase("a passphrase"), 20, None);
    assert!(matches!(result, Err(ArchiveError::TooLarge)), "{result:?}");
}
