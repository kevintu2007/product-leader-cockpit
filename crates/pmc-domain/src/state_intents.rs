//! What each record's current state admits.
//!
//! The Work Queue (S03) must preserve "legal actions", and DG0 lists
//! "legal next intent" as Work Queue content. Nothing enumerated them: the
//! legal transitions existed only as *rejections* -- guards scattered through
//! the service methods that return an illegal-transition error.
//!
//! Every table below was read from those guards rather than inferred from
//! method names, and each cites where. Two of the five already existed as
//! private diagnostics inside `actions`, used to populate safe errors; they
//! are reused rather than restated, because a second copy of a state machine
//! drifts from the one that enforces it, and drift here means a surface
//! offering an action the domain will refuse.
//!
//! **These are not permissions.** A state-admissible intent is strictly
//! weaker than an executable one: preparation can still deny missing
//! Evidence, and classification, policy and approval are further gates that
//! DG3 requires H2 execution to revalidate. A surface may present these as
//! what is not excluded by the record's state, never as what may be done now.
//!
//! Every function is an exhaustive match, so a new state fails to compile
//! here rather than silently returning nothing and hiding a real next step.

use crate::work_management::{DecisionRequestState, IssueState, RiskState};

/// Decision Request transitions.
///
/// Read from `decisions.rs`: `submit_decision_request` transitions
/// `Draft -> Open`, `withdraw_decision_request` transitions
/// `Open -> Withdrawn`, and resolution requires `Open`.
#[must_use]
pub const fn decision_request_state_intents(
    state: DecisionRequestState,
) -> &'static [&'static str] {
    match state {
        DecisionRequestState::Draft => &["submit_decision_request"],
        DecisionRequestState::Open => &[
            "withdraw_decision_request",
            "prepare_resolve_decision_request",
        ],
        // Both terminal: nothing further is admitted.
        DecisionRequestState::Resolved | DecisionRequestState::Withdrawn => &[],
    }
}

/// Risk transitions.
///
/// Read from `risks.rs`: both `prepare_occurrence` and `prepare_close`
/// require `RiskState::Open`. An Occurred Risk therefore admits no further
/// lifecycle transition of its own -- the occurrence is what creates an
/// Issue, and the Issue carries the work from there.
#[must_use]
pub const fn risk_state_intents(state: RiskState) -> &'static [&'static str] {
    match state {
        RiskState::Open => &["prepare_record_risk_occurrence", "prepare_close_risk"],
        RiskState::Occurred | RiskState::Closed => &[],
    }
}

/// Issue transitions.
///
/// Read from the explicit required-state mapping in `issues.rs`:
/// `ResolveIssue` requires `Open`, and both `CloseIssue` and `ReopenIssue`
/// require `Resolved`. Closing therefore goes through resolution rather than
/// directly from `Open`.
#[must_use]
pub const fn issue_state_intents(state: IssueState) -> &'static [&'static str] {
    match state {
        IssueState::Open => &["prepare_resolve_issue"],
        IssueState::Resolved => &["prepare_close_issue", "prepare_reopen_issue"],
        IssueState::Closed => &[],
    }
}
