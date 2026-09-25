//! What each record's current state admits.
//!
//! These tables are only worth having if they equal what the service actually
//! enforces. A table that merely looks right is worse than none: a surface
//! would offer an action the domain refuses, or hide one it would allow.
//!
//! Two kinds of check here. The first pins each table against the guard it was
//! read from, naming the guard, so a change to either side without the other
//! shows up as a failure rather than as drift. The second holds properties
//! that must be true of all five tables at once.

use pmc_domain::actions::{action_allowed_intents, request_allowed_intents};
use pmc_domain::state_intents::{
    decision_request_state_intents, issue_state_intents, risk_state_intents,
};
use pmc_domain::work_management::{
    ActionRequestState, ActionState, DecisionRequestState, IssueState, RiskState,
};

/// Every state of all five aggregates, so a property test cannot silently
/// skip one.
fn all_states() -> Vec<(&'static str, &'static [&'static str])> {
    let mut rows: Vec<(&'static str, &'static [&'static str])> = Vec::new();
    for state in [
        ActionRequestState::Draft,
        ActionRequestState::Open,
        ActionRequestState::Accepted,
        ActionRequestState::Declined,
        ActionRequestState::Withdrawn,
    ] {
        rows.push(("action_request", request_allowed_intents(state)));
    }
    for state in [
        ActionState::Open,
        ActionState::InProgress,
        ActionState::Completed,
        ActionState::Cancelled,
    ] {
        rows.push(("action", action_allowed_intents(state)));
    }
    for state in [
        DecisionRequestState::Draft,
        DecisionRequestState::Open,
        DecisionRequestState::Resolved,
        DecisionRequestState::Withdrawn,
    ] {
        rows.push(("decision_request", decision_request_state_intents(state)));
    }
    for state in [RiskState::Open, RiskState::Occurred, RiskState::Closed] {
        rows.push(("risk", risk_state_intents(state)));
    }
    for state in [IssueState::Open, IssueState::Resolved, IssueState::Closed] {
        rows.push(("issue", issue_state_intents(state)));
    }
    rows
}

#[test]
fn a_decision_request_admits_exactly_what_its_guards_allow() {
    // `submit_decision_request` transitions Draft -> Open;
    // `withdraw_decision_request` transitions Open -> Withdrawn; resolution
    // requires Open. Both terminal states admit nothing.
    assert_eq!(
        decision_request_state_intents(DecisionRequestState::Draft),
        &["submit_decision_request"]
    );
    assert_eq!(
        decision_request_state_intents(DecisionRequestState::Open),
        &[
            "withdraw_decision_request",
            "prepare_resolve_decision_request"
        ]
    );
    assert!(decision_request_state_intents(DecisionRequestState::Resolved).is_empty());
    assert!(decision_request_state_intents(DecisionRequestState::Withdrawn).is_empty());
}

#[test]
fn a_risk_admits_nothing_once_it_has_occurred_or_closed() {
    // Both `prepare_occurrence` and `prepare_close` require `RiskState::Open`.
    // An Occurred Risk therefore has no lifecycle transition of its own --
    // the occurrence creates an Issue, and the Issue carries the work.
    assert_eq!(
        risk_state_intents(RiskState::Open),
        &["prepare_record_risk_occurrence", "prepare_close_risk"]
    );
    assert!(risk_state_intents(RiskState::Occurred).is_empty());
    assert!(risk_state_intents(RiskState::Closed).is_empty());
}

#[test]
fn an_issue_is_closed_through_resolution_rather_than_directly() {
    // The required-state mapping in `issues.rs`: ResolveIssue requires Open,
    // and both CloseIssue and ReopenIssue require Resolved. So an Open Issue
    // cannot be closed without first being resolved -- offering "close" on an
    // Open Issue would be offering an action the domain refuses.
    assert_eq!(
        issue_state_intents(IssueState::Open),
        &["prepare_resolve_issue"]
    );
    assert!(!issue_state_intents(IssueState::Open).contains(&"prepare_close_issue"));
    assert_eq!(
        issue_state_intents(IssueState::Resolved),
        &["prepare_close_issue", "prepare_reopen_issue"]
    );
    assert!(issue_state_intents(IssueState::Closed).is_empty());
}

#[test]
fn the_reused_action_tables_are_the_ones_the_safe_errors_use() {
    // Exposed rather than restated. If these ever diverge from the safe-error
    // diagnostics, a surface and an error message would describe different
    // sets of next steps for the same record.
    assert_eq!(
        request_allowed_intents(ActionRequestState::Draft),
        &["submit_action_request"]
    );
    assert!(request_allowed_intents(ActionRequestState::Accepted).is_empty());
    assert_eq!(
        action_allowed_intents(ActionState::Open),
        &["start_action", "prepare_cancel_action"]
    );
    // A cancelled or completed Action can be reopened, so neither is a dead
    // end even though both are terminal states of the forward path.
    assert_eq!(
        action_allowed_intents(ActionState::Cancelled),
        &["prepare_reopen_action"]
    );
    assert_eq!(
        action_allowed_intents(ActionState::Completed),
        &["prepare_reopen_action"]
    );
}

#[test]
fn no_state_offers_the_same_intent_twice() {
    // A duplicate would show a reader the same next step twice and make any
    // count of available actions wrong.
    for (aggregate, intents) in all_states() {
        let mut seen = std::collections::HashSet::new();
        for intent in intents {
            assert!(
                seen.insert(*intent),
                "{aggregate} offers {intent} more than once in one state"
            );
        }
    }
}

#[test]
fn every_intent_is_named_as_a_service_method_not_a_type_or_a_message_key() {
    // Three naming forms exist in this codebase: Rust command types
    // (`PrepareCompleteAction`), service methods (`prepare_complete_action`),
    // and message keys (`issue.prepare_resolve`). The tables use service
    // method names because the two pre-existing tables already did, and one
    // vocabulary across every surface beats a translation layer.
    for (aggregate, intents) in all_states() {
        for intent in intents {
            assert!(
                !intent.contains('.'),
                "{aggregate}: {intent} looks like a message key"
            );
            assert!(
                !intent.chars().next().is_some_and(char::is_uppercase),
                "{aggregate}: {intent} looks like a type name"
            );
            assert!(
                intent
                    .chars()
                    .all(|character| character.is_ascii_lowercase() || character == '_'),
                "{aggregate}: {intent} is not a snake_case service method name"
            );
        }
    }
}

#[test]
fn a_terminal_state_offers_nothing_unless_it_can_genuinely_be_reopened() {
    // The only terminal states that offer anything are Action's, because
    // reopen is a real transition out of them. Every other terminal state
    // offering something would mean the table promises a transition the
    // service does not implement.
    assert!(request_allowed_intents(ActionRequestState::Declined).is_empty());
    assert!(request_allowed_intents(ActionRequestState::Withdrawn).is_empty());
    assert!(decision_request_state_intents(DecisionRequestState::Resolved).is_empty());
    assert!(risk_state_intents(RiskState::Closed).is_empty());
    assert!(issue_state_intents(IssueState::Closed).is_empty());

    assert!(!action_allowed_intents(ActionState::Cancelled).is_empty());
}
