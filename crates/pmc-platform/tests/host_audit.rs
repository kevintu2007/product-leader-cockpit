//! The host audit log of ADR 0012: append-only, flushed, and a torn last
//! line never swallows the next event.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_platform::host_audit::{
    AuditOutcome, AuditWorkspace, HostAuditEvent, HostAuditLog, BACKUP_COMPLETED,
    BOOTSTRAP_EMPTY_AUTHORITY,
};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn log_path(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir()
        .join(format!("pmc-host-audit-{name}-{nonce}-{sequence}"))
        .join("host-audit-v1.jsonl")
}

fn event(id: &str, code: &str) -> HostAuditEvent {
    HostAuditEvent::new(
        id.to_owned(),
        1_000,
        AuditWorkspace::Live,
        code,
        AuditOutcome::Succeeded,
        vec![("record_count".to_owned(), "0".to_owned())],
    )
}

#[test]
fn events_are_appended_in_order_and_read_back() {
    let log = HostAuditLog::new(log_path("order"));
    assert!(log
        .events()
        .unwrap_or_else(|error| panic!("{error}"))
        .is_empty());
    log.append(&event("e1", BOOTSTRAP_EMPTY_AUTHORITY))
        .unwrap_or_else(|error| panic!("{error}"));
    log.append(&event("e2", BACKUP_COMPLETED))
        .unwrap_or_else(|error| panic!("{error}"));
    let events = log.events().unwrap_or_else(|error| panic!("{error}"));
    let ids: Vec<&str> = events.iter().map(|event| event.event_id.as_str()).collect();
    assert_eq!(ids, ["e1", "e2"]);
    assert!(log
        .has_event(AuditWorkspace::Live, BOOTSTRAP_EMPTY_AUTHORITY)
        .unwrap_or_else(|error| panic!("{error}")));
    assert!(!log
        .has_event(AuditWorkspace::Training, BOOTSTRAP_EMPTY_AUTHORITY)
        .unwrap_or_else(|error| panic!("{error}")));
}

#[test]
fn a_torn_last_line_is_kept_and_does_not_swallow_the_next_event() {
    let path = log_path("torn");
    let log = HostAuditLog::new(path.clone());
    log.append(&event("e1", BACKUP_COMPLETED))
        .unwrap_or_else(|error| panic!("{error}"));
    // A crash mid-append: half a record, no newline.
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .and_then(|mut file| file.write_all(br#"{"format":"pmc-host-audit/v1","event_id":"to"#))
        .unwrap_or_else(|error| panic!("{error}"));
    log.append(&event("e3", BACKUP_COMPLETED))
        .unwrap_or_else(|error| panic!("{error}"));
    let ids: Vec<String> = log
        .events()
        .unwrap_or_else(|error| panic!("{error}"))
        .into_iter()
        .map(|event| event.event_id)
        .collect();
    assert_eq!(ids, ["e1", "e3"]);
    let text = fs::read_to_string(&path).unwrap_or_else(|error| panic!("{error}"));
    assert!(
        text.contains(r#""event_id":"to"#),
        "the torn record is kept"
    );
    assert!(text.ends_with('\n'));
}
