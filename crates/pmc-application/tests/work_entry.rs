//! Record entry for work (slice 6E) through the host's flows: an Action
//! Request draft named by its reservation and submitted at the version read,
//! a Decision Request draft likewise, and an Issue; each retry from another
//! launch names the same record.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_application::desktop_runtime::OpaqueIdSource;
use pmc_application::work_entry::{
    enter_action_request_draft, enter_decision_request_draft, enter_issue, submit_action_request,
    submit_decision_request,
};
use pmc_domain::actions::{ActionDetails, ActionOperationContext, ActionTitle};
use pmc_domain::classification::DataClassification;
use pmc_domain::decisions::{DecisionOperationContext, DecisionSubject, DecisionText};
use pmc_domain::error::ErrorCode;
use pmc_domain::identity::{CorrelationId, IdempotencyId};
use pmc_domain::issues::{IssueDetails, IssueOperationContext, IssueTitle};
use pmc_domain::time::UtcTimestamp;
use pmc_ledger::sqlite::{
    ActionRequestDraftFields, DecisionRequestDraftFields, IssueFields, SqliteProductLedger,
};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn ledger_path() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("synthetic test clock must follow the Unix epoch")
        .as_nanos();
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("pmc-work-entry-{nonce}-{sequence}.sqlite3"))
}

fn ids(request: &str) -> (IdempotencyId, CorrelationId) {
    (
        IdempotencyId::parse(request).unwrap_or_else(|_| panic!("id")),
        CorrelationId::parse(format!("corr-{request}")).unwrap_or_else(|_| panic!("id")),
    )
}

fn action_context(request: &str) -> ActionOperationContext {
    let (idempotency_id, correlation_id) = ids(request);
    ActionOperationContext {
        idempotency_id,
        correlation_id,
    }
}

fn decision_context(request: &str) -> DecisionOperationContext {
    let (idempotency_id, correlation_id) = ids(request);
    DecisionOperationContext {
        idempotency_id,
        correlation_id,
    }
}

fn issue_context(request: &str) -> IssueOperationContext {
    let (idempotency_id, correlation_id) = ids(request);
    IssueOperationContext {
        idempotency_id,
        correlation_id,
    }
}

fn at(millis: i64) -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(millis)
}

fn action_request() -> ActionRequestDraftFields {
    ActionRequestDraftFields {
        title: ActionTitle::parse("Confirm the window").unwrap_or_else(|_| panic!("text")),
        details: ActionDetails::parse("Synthetic only.").unwrap_or_else(|_| panic!("text")),
        intended_owner: None,
        response_due_at: Some(at(1_700_500_000_000)),
        intended_action_due_at: None,
        classification: DataClassification::Internal,
    }
}

#[test]
fn a_draft_is_named_by_its_reservation_then_submitted_at_the_version_read() {
    let path = ledger_path();
    let mut ledger = SqliteProductLedger::open(&path).unwrap_or_else(|e| panic!("{e:?}"));
    let mut minter = OpaqueIdSource::with_nonce(41);
    let draft = enter_action_request_draft(
        &mut ledger,
        action_request(),
        action_context("ar1"),
        &mut minter,
        at(1),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .record;
    assert!(draft.id().as_str().starts_with("request-"));
    assert_eq!(draft.state().as_persisted(), "draft");

    // A retry from another launch names the same draft.
    drop(ledger);
    let mut ledger = SqliteProductLedger::open(&path).unwrap_or_else(|e| panic!("{e:?}"));
    let mut later = OpaqueIdSource::with_nonce(42);
    let again = enter_action_request_draft(
        &mut ledger,
        action_request(),
        action_context("ar1"),
        &mut later,
        at(2),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .record;
    assert_eq!(again.id(), draft.id());
    assert_eq!(again.version(), draft.version());

    // Submit at the version read: Draft -> Open; a second submit at the old
    // version is stale.
    let open = submit_action_request(
        &mut ledger,
        draft.id().clone(),
        draft.version(),
        action_context("ar1-submit"),
        &mut later,
        at(3),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .record;
    assert_eq!(open.state().as_persisted(), "open");
    assert!(open.version() > draft.version());
    let stale = submit_action_request(
        &mut ledger,
        draft.id().clone(),
        draft.version(),
        action_context("ar1-submit-again"),
        &mut later,
        at(4),
    );
    match stale {
        Err(error) => assert_eq!(error.code(), ErrorCode::DomainConflict),
        Ok(_) => panic!("a stale submit must be refused"),
    }
}

#[test]
fn a_decision_request_draft_and_an_issue_are_entered_and_the_draft_submitted() {
    let mut ledger = SqliteProductLedger::open(ledger_path()).unwrap_or_else(|e| panic!("{e:?}"));
    let mut minter = OpaqueIdSource::with_nonce(43);
    let draft = enter_decision_request_draft(
        &mut ledger,
        DecisionRequestDraftFields {
            subject: DecisionSubject::parse("Pricing").unwrap_or_else(|_| panic!("text")),
            details: DecisionText::parse("Which tier?").unwrap_or_else(|_| panic!("text")),
            intended_owner: None,
            classification: DataClassification::Internal,
        },
        decision_context("dr1"),
        &mut minter,
        at(1),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .record;
    assert!(draft.id().as_str().starts_with("decision-request-"));
    let open = submit_decision_request(
        &mut ledger,
        draft.id().clone(),
        draft.version(),
        decision_context("dr1-submit"),
        &mut minter,
        at(2),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .record;
    assert_eq!(open.state().as_persisted(), "open");

    let issue = enter_issue(
        &mut ledger,
        IssueFields {
            title: IssueTitle::parse("Export broken").unwrap_or_else(|_| panic!("text")),
            details: IssueDetails::parse("Synthetic only.").unwrap_or_else(|_| panic!("text")),
            classification: DataClassification::Confidential,
            recurrence_of: None,
        },
        issue_context("i1"),
        &mut minter,
        at(3),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .record;
    assert!(issue.id().as_str().starts_with("issue-"));
    assert_eq!(issue.classification(), DataClassification::Confidential);
}
