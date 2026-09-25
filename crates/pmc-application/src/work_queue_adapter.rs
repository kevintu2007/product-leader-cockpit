//! The production Work Queue (S03) adapter.
//!
//! Maps two real Ledger snapshots into the [`WorkItemFacts`] that
//! [`crate::work_queue_composition`] orders. The five lifecycle types do not
//! come from one place: Actions and Risks are in the managed-projection
//! snapshot, and Action Requests, Decision Requests and Issues are in the
//! composition snapshot. Both are read here so that no surface has to know
//! which port a given type lives behind.
//!
//! # Legal intents are read, not written
//!
//! Every item's intents come from `pmc_domain::state_intents` and the two
//! tables `pmc_domain::actions` already uses for its safe errors. There is no
//! table in this module. A state machine copied into an adapter drifts from
//! the guards that enforce it, and drift here means the queue offering an
//! action the domain will refuse.
//!
//! # What attention this adapter can and cannot derive
//!
//! `derive_attention` needs facts the read surfaces do not all carry. Where a
//! fact is missing, the input is set in the direction that produces **no
//! flag**, never in the direction that produces one:
//!
//! | Input not carried | Set to | Consequence |
//! |---|---|---|
//! | Action `blocked`, `at_risk`, `evidence_state`, `superseded_premise` | absent | no blocked / at-risk / evidence attention for Actions |
//! | Action Request `needs_info` | `false` | no needs-info attention |
//! | Decision Request `decision_deadline_at` | `None` | no deadline attention; the Ledger persists no decision deadline at all |
//! | Decision Request `needs_info` | `false` | no needs-info attention |
//! | Risk `exposure_increased`, `evidence_stale`, `control_invalid` | `false` | no exposure or evidence-integrity attention for Risks |
//! | Risk `owner_present` | **`true`** | no missing-owner attention for Risks |
//! | Issue `resolution_due_at` | `None` | no resolution-due or overdue attention |
//! | Issue `blocked`, resolution evidence | absent | no blocked or evidence attention for Issues |
//!
//! The Risk row is the one that needs explaining, because it is the only
//! unknown set to `true`. `owner_present: false` is not a safe default: it
//! **creates** a `RiskMissingOwner` flag, which would put a Risk near the top
//! of the queue for a reason that may not be true. The other unknowns are
//! safe at `false` precisely because `false` produces silence there. So the
//! rule applied throughout is not "default to false", it is "default to
//! whichever value asserts nothing" -- and that value differs per field.
//!
//! Both directions of error are real. Suppressing an underivable flag can
//! miss something; fabricating one states something false. This adapter
//! chooses to miss rather than to fabricate, and says here exactly what it
//! misses, so the gap is visible rather than silently absorbed.
//!
//! Owner presence for Action Requests and Decision Requests **is** carried by
//! the composition snapshot, so their missing-owner attention is genuinely
//! derived and is not in the table above.
//!
//! Nothing here writes. Evaluating attention is a pure derivation that cannot
//! mutate lifecycle state, advance a clock, or create audit records.

use pmc_domain::actions::{action_allowed_intents, request_allowed_intents};
use pmc_domain::attention::{
    derive_attention, ActionAttentionInput, ActionRequestAttentionInput, AttentionInputs,
    AttentionMetadata, AttentionTarget, AttentionThresholds, DecisionRequestAttentionInput,
    EvidenceAttentionState, Freshness, IssueAttentionInput, ResolvedIssueVerification,
    RiskAttentionInput,
};
use pmc_domain::classification::DataClassification;
use pmc_domain::composition_source::LedgerCompositionSnapshot;
use pmc_domain::projection_source::LedgerProjectionSnapshot;
use pmc_domain::state_intents::{
    decision_request_state_intents, issue_state_intents, risk_state_intents,
};
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::{
    ActionRequestState, ActionState, DecisionRequestState, IssueState, RiskState,
};

use crate::attention_ranking::{RankableAttentionItem, RankedAttentionItem};
use crate::route_composition::PageState;
use crate::work_queue_composition::{
    compose_work_queue, GetWorkQueue, WorkItemFacts, WorkItemKind, WorkQueueFilter,
};

const fn action_request_state_label(state: ActionRequestState) -> &'static str {
    match state {
        ActionRequestState::Draft => "Draft",
        ActionRequestState::Open => "Open",
        ActionRequestState::Accepted => "Accepted",
        ActionRequestState::Declined => "Declined",
        ActionRequestState::Withdrawn => "Withdrawn",
    }
}

const fn action_state_label(state: ActionState) -> &'static str {
    match state {
        ActionState::Open => "Open",
        ActionState::InProgress => "In progress",
        ActionState::Completed => "Completed",
        ActionState::Cancelled => "Cancelled",
    }
}

const fn decision_request_state_label(state: DecisionRequestState) -> &'static str {
    match state {
        DecisionRequestState::Draft => "Draft",
        DecisionRequestState::Open => "Open",
        DecisionRequestState::Resolved => "Resolved",
        DecisionRequestState::Withdrawn => "Withdrawn",
    }
}

const fn risk_state_label(state: RiskState) -> &'static str {
    match state {
        RiskState::Open => "Open",
        RiskState::Occurred => "Occurred",
        RiskState::Closed => "Closed",
    }
}

const fn issue_state_label(state: IssueState) -> &'static str {
    match state {
        IssueState::Open => "Open",
        IssueState::Resolved => "Resolved",
        IssueState::Closed => "Closed",
    }
}

fn metadata(classification: DataClassification) -> AttentionMetadata {
    AttentionMetadata {
        classification,
        // The snapshot is read in one transaction at a known revision, so its
        // rows are current as of that read. Freshness that varies per record
        // would need a per-record read time the Ledger does not return.
        freshness: Freshness::Fresh,
        degraded: false,
    }
}

/// Builds the attention inputs both snapshots can genuinely support.
///
/// See the module note for the fields that have no source and the direction
/// each was set in.
fn attention_inputs(
    projection: &LedgerProjectionSnapshot,
    composition: &LedgerCompositionSnapshot,
    thresholds: AttentionThresholds,
) -> AttentionInputs {
    AttentionInputs {
        as_of: composition.ledger_as_of_utc,
        thresholds,
        action_requests: composition
            .action_requests
            .iter()
            .filter_map(|request| {
                Some(ActionRequestAttentionInput {
                    id: pmc_domain::identity::ActionRequestId::parse(&request.id).ok()?,
                    state: request.state,
                    intended_owner_present: request.intended_owner_id.is_some(),
                    response_due_at: request.response_due_at,
                    needs_info: false,
                    metadata: metadata(request.classification),
                })
            })
            .collect(),
        actions: projection
            .actions
            .iter()
            .map(|action| ActionAttentionInput {
                id: action.id.clone(),
                state: action.state,
                due_at: action.due_at,
                blocked: false,
                at_risk: false,
                evidence_state: EvidenceAttentionState::None,
                superseded_premise: false,
                metadata: metadata(action.classification),
            })
            .collect(),
        decision_requests: composition
            .decision_requests
            .iter()
            .filter_map(|request| {
                Some(DecisionRequestAttentionInput {
                    id: pmc_domain::identity::DecisionRequestId::parse(&request.id).ok()?,
                    state: request.state,
                    decision_owner_present: request.intended_owner_id.is_some(),
                    // The Ledger persists no decision deadline anywhere. Not
                    // defaulted to a date: see the membership-and-ordering policy §4.
                    decision_deadline_at: None,
                    needs_info: false,
                    metadata: metadata(request.classification),
                })
            })
            .collect(),
        risks: projection
            .risks
            .iter()
            .map(|risk| {
                RiskAttentionInput::in_queue(
                    risk.id.clone(),
                    risk.state,
                    risk.next_review_at,
                    false,
                    false,
                    false,
                    // `true` asserts nothing here; `false` would fabricate a
                    // missing-owner flag. See the module note.
                    true,
                    metadata(risk.classification),
                )
            })
            .collect(),
        issues: composition
            .issues
            .iter()
            .filter_map(|issue| {
                let id = pmc_domain::identity::IssueId::parse(&issue.id).ok()?;
                let recurrence = issue.recurrence_of_id.is_some();
                Some(match issue.state {
                    IssueState::Open => IssueAttentionInput::open(
                        id,
                        None,
                        false,
                        false,
                        recurrence,
                        metadata(issue.classification),
                    ),
                    IssueState::Resolved | IssueState::Closed => IssueAttentionInput::resolved(
                        id,
                        None,
                        false,
                        // Asserts nothing. `Pending` or `EvidenceMissing`
                        // would raise an evidence-integrity flag this
                        // adapter cannot substantiate.
                        ResolvedIssueVerification::Complete,
                        recurrence,
                        metadata(issue.classification),
                    ),
                })
            })
            .collect(),
    }
}

/// Every flag the matched snapshots support, each paired with the kind and
/// identifier of the record it was raised against and that record's deadline.
///
/// The deadline comes from the same snapshot record the flag was derived from,
/// so a rank is never computed against a deadline that disagrees with the fact
/// beside it. This is the single derivation the Work Queue and the Cockpit
/// both stand on.
fn flags_from_snapshots(
    projection: &LedgerProjectionSnapshot,
    composition: &LedgerCompositionSnapshot,
    thresholds: AttentionThresholds,
) -> Vec<(WorkItemKind, String, RankableAttentionItem)> {
    derive_attention(&attention_inputs(projection, composition, thresholds))
        .flags
        .into_iter()
        .map(|flag| {
            let kind = WorkItemKind::of_target(&flag.target);
            let id = target_id(&flag.target).to_owned();
            let relevant_at = relevant_at_for(&kind, &id, projection, composition);
            (kind, id, RankableAttentionItem { flag, relevant_at })
        })
        .collect()
}

/// Every attention flag across the Portfolio, in the order the Work Queue
/// shows them.
///
/// The Cockpit's exception list. It is the flagged Work Queue itself,
/// flattened: the same records in the same order, each record's flags together
/// and ranked by the accepted policy. Ranking all flags globally instead would
/// split one record's reasons across the list and make the Cockpit read in a
/// different order from the Work Queue for the same facts (DG3 S01 amendment
/// §7). The snapshots must share a Ledger revision; as with the Work Queue,
/// only the caller holds both ports and can check that.
#[must_use]
pub fn portfolio_attention_from_snapshots(
    projection: &LedgerProjectionSnapshot,
    composition: &LedgerCompositionSnapshot,
    thresholds: AttentionThresholds,
) -> Vec<RankedAttentionItem> {
    let facts = work_item_facts_from_snapshots(projection, composition, thresholds);
    let flagged = compose_work_queue(
        &GetWorkQueue {
            as_of: composition.ledger_as_of_utc,
            page: PageState {
                offset: 0,
                limit: usize::MAX,
                total: 0,
            },
            filter: WorkQueueFilter {
                kinds: Vec::new(),
                only_flagged: true,
            },
        },
        &facts,
        composition.ledger_revision,
    );
    flagged
        .body
        .items
        .into_iter()
        .flat_map(|item| item.attention)
        .collect()
}

/// The identifier a flag was raised against, so flags can be attached to the
/// item they belong to.
fn target_id(target: &AttentionTarget) -> &str {
    crate::attention_ranking::canonical_id_of(target)
}

/// Builds every Work Queue item from a matched pair of Ledger snapshots.
///
/// The two snapshots must have been read at the same Ledger revision. That is
/// the caller's to guarantee, because only the caller holds both ports; this
/// function reports the composition snapshot's revision so a mismatch is
/// visible rather than silently averaged.
#[must_use]
pub fn work_item_facts_from_snapshots(
    projection: &LedgerProjectionSnapshot,
    composition: &LedgerCompositionSnapshot,
    thresholds: AttentionThresholds,
) -> Vec<WorkItemFacts> {
    let flags = flags_from_snapshots(projection, composition, thresholds);

    let attention_for = |kind: WorkItemKind, id: &str| -> Vec<RankableAttentionItem> {
        flags
            .iter()
            .filter(|(flag_kind, flag_id, _)| *flag_kind == kind && flag_id == id)
            .map(|(_, _, item)| item.clone())
            .collect()
    };

    let mut facts = Vec::new();

    for request in &composition.action_requests {
        facts.push(WorkItemFacts {
            kind: WorkItemKind::ActionRequest,
            id: request.id.clone(),
            label: request.title.clone(),
            state_label: action_request_state_label(request.state),
            lifecycle_legal_intents: request_allowed_intents(request.state).to_vec(),
            relevant_at: request.response_due_at,
            promised_at: request.intended_action_due_at,
            classification: request.classification,
            revision: request.version.get(),
            as_of: composition.ledger_as_of_utc,
            freshness: Freshness::Fresh,
            degraded: false,
            attention: attention_for(WorkItemKind::ActionRequest, &request.id),
        });
    }

    for action in &projection.actions {
        let id = action.id.as_str().to_owned();
        facts.push(WorkItemFacts {
            kind: WorkItemKind::Action,
            // The projection snapshot carries no title for an Action. The
            // type and identifier are true and unhelpful rather than helpful
            // and invented; when the Ledger grows a title on this surface the
            // label improves without the contract changing.
            label: format!("Action {id}"),
            state_label: action_state_label(action.state),
            lifecycle_legal_intents: action_allowed_intents(action.state).to_vec(),
            relevant_at: Some(action.due_at),
            promised_at: None,
            classification: action.classification,
            revision: action.source_revision.get(),
            as_of: projection.ledger_as_of_utc,
            freshness: Freshness::Fresh,
            degraded: false,
            attention: attention_for(WorkItemKind::Action, &id),
            id,
        });
    }

    for request in &composition.decision_requests {
        facts.push(WorkItemFacts {
            kind: WorkItemKind::DecisionRequest,
            id: request.id.clone(),
            label: request.subject.clone(),
            state_label: decision_request_state_label(request.state),
            lifecycle_legal_intents: decision_request_state_intents(request.state).to_vec(),
            // No decision deadline exists in the Ledger. Never defaulted.
            relevant_at: None,
            promised_at: None,
            classification: request.classification,
            revision: request.version.get(),
            as_of: composition.ledger_as_of_utc,
            freshness: Freshness::Fresh,
            degraded: false,
            attention: attention_for(WorkItemKind::DecisionRequest, &request.id),
        });
    }

    for risk in &projection.risks {
        let id = risk.id.as_str().to_owned();
        // The title comes from the composition snapshot, read at its own
        // instant; a Risk created between the two reads has no title yet and
        // keeps the true, unhelpful identifier rather than an invented name.
        let label = composition
            .risks
            .iter()
            .find(|titled| titled.id == id)
            .map_or_else(|| format!("Risk {id}"), |titled| titled.title.clone());
        facts.push(WorkItemFacts {
            kind: WorkItemKind::Risk,
            label,
            state_label: risk_state_label(risk.state),
            lifecycle_legal_intents: risk_state_intents(risk.state).to_vec(),
            relevant_at: risk.next_review_at,
            promised_at: None,
            classification: risk.classification,
            revision: risk.source_revision.get(),
            as_of: projection.ledger_as_of_utc,
            freshness: Freshness::Fresh,
            degraded: false,
            attention: attention_for(WorkItemKind::Risk, &id),
            id,
        });
    }

    for issue in &composition.issues {
        facts.push(WorkItemFacts {
            kind: WorkItemKind::Issue,
            id: issue.id.clone(),
            label: issue.title.clone(),
            state_label: issue_state_label(issue.state),
            lifecycle_legal_intents: issue_state_intents(issue.state).to_vec(),
            // No issue resolution deadline exists in the Ledger read surface.
            relevant_at: None,
            promised_at: None,
            classification: issue.classification,
            revision: issue.version.get(),
            as_of: composition.ledger_as_of_utc,
            freshness: Freshness::Fresh,
            degraded: false,
            attention: attention_for(WorkItemKind::Issue, &issue.id),
        });
    }

    facts
}

/// The deadline belonging to the record a flag was raised against.
fn relevant_at_for(
    kind: &WorkItemKind,
    id: &str,
    projection: &LedgerProjectionSnapshot,
    composition: &LedgerCompositionSnapshot,
) -> Option<UtcTimestamp> {
    match kind {
        WorkItemKind::ActionRequest => composition
            .action_requests
            .iter()
            .find(|request| request.id == id)
            .and_then(|request| request.response_due_at),
        WorkItemKind::Action => projection
            .actions
            .iter()
            .find(|action| action.id.as_str() == id)
            .map(|action| action.due_at),
        WorkItemKind::Risk => projection
            .risks
            .iter()
            .find(|risk| risk.id.as_str() == id)
            .and_then(|risk| risk.next_review_at),
        // Neither is persisted with a deadline anywhere in the Ledger.
        WorkItemKind::DecisionRequest | WorkItemKind::Issue => None,
    }
}
