//! Pure, read-only derivation of attention flags.
//!
//! The evaluator deliberately accepts snapshots rather than stores.  This keeps
//! attention a query concern: evaluating it cannot mutate lifecycle state,
//! advance a clock, or create audit records.

use crate::classification::DataClassification;
use crate::identity::StakeholderId;
use crate::identity::{ActionId, ActionRequestId, DecisionRequestId, IssueId, RiskId};
use crate::risks::{ResidualExposure, RiskRationale};
use crate::time::UtcTimestamp;
use crate::work_management::{
    ActionRequestState, ActionState, DecisionRequestState, IssueState, RiskState,
};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum AttentionReason {
    ActionRequestNeedsInfo,
    ActionRequestStale,
    ActionRequestMissingIntendedOwner,
    ActionRequestResponseDue,
    ActionRequestResponseOverdue,
    ActionBlocked,
    ActionOverdue,
    ActionAtRisk,
    ActionNeedsEvidence,
    ActionEvidenceVerificationPending,
    ActionSupersededPremise,
    DecisionRequestNeedsInfo,
    DecisionRequestMissingDecisionOwner,
    DecisionRequestApproachingDeadline,
    DecisionRequestOverdue,
    DecisionRequestStale,
    RiskReviewDue,
    RiskExposureIncreased,
    RiskEvidenceStale,
    RiskControlInvalid,
    RiskMissingOwner,
    IssueBlocked,
    IssueResolutionDue,
    IssueResolutionOverdue,
    IssueNeedsEvidence,
    IssueStale,
    IssueRecurrence,
}

impl AttentionReason {
    /// A stable identifier for the reason.
    ///
    /// The Work Queue (S03) must preserve attention flags, and DG0
    /// lists "attention reasons" as Work Queue content. A surface therefore
    /// has to name the reason, and it must do so without depending on
    /// anything that can shift underneath it: the `Debug` rendering is a
    /// derived convenience that is not a contract, and the explanation text
    /// is user-visible prose that may be reworded. This is neither.
    ///
    /// It is deliberately NOT an ordering. The accepted ranking policy §5
    /// refuses `AttentionReason`'s derived `Ord` because declaration order is
    /// an accident of how the enum was written; naming a reason says nothing
    /// about its severity, which `AttentionTier` alone decides.
    ///
    /// Exhaustive: a twenty-eighth reason will not compile until it is named
    /// here, so a new reason cannot reach a surface as an empty string or a
    /// silent fallback.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ActionRequestNeedsInfo => "action_request_needs_info",
            Self::ActionRequestStale => "action_request_stale",
            Self::ActionRequestMissingIntendedOwner => "action_request_missing_intended_owner",
            Self::ActionRequestResponseDue => "action_request_response_due",
            Self::ActionRequestResponseOverdue => "action_request_response_overdue",
            Self::ActionBlocked => "action_blocked",
            Self::ActionOverdue => "action_overdue",
            Self::ActionAtRisk => "action_at_risk",
            Self::ActionNeedsEvidence => "action_needs_evidence",
            Self::ActionEvidenceVerificationPending => "action_evidence_verification_pending",
            Self::ActionSupersededPremise => "action_superseded_premise",
            Self::DecisionRequestNeedsInfo => "decision_request_needs_info",
            Self::DecisionRequestMissingDecisionOwner => "decision_request_missing_decision_owner",
            Self::DecisionRequestApproachingDeadline => "decision_request_approaching_deadline",
            Self::DecisionRequestOverdue => "decision_request_overdue",
            Self::DecisionRequestStale => "decision_request_stale",
            Self::RiskReviewDue => "risk_review_due",
            Self::RiskExposureIncreased => "risk_exposure_increased",
            Self::RiskEvidenceStale => "risk_evidence_stale",
            Self::RiskControlInvalid => "risk_control_invalid",
            Self::RiskMissingOwner => "risk_missing_owner",
            Self::IssueBlocked => "issue_blocked",
            Self::IssueResolutionDue => "issue_resolution_due",
            Self::IssueResolutionOverdue => "issue_resolution_overdue",
            Self::IssueNeedsEvidence => "issue_needs_evidence",
            Self::IssueStale => "issue_stale",
            Self::IssueRecurrence => "issue_recurrence",
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum AttentionTarget {
    ActionRequest(ActionRequestId),
    Action(ActionId),
    DecisionRequest(DecisionRequestId),
    Risk(RiskId),
    Issue(IssueId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Freshness {
    Fresh,
    Stale,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceAttentionState {
    None,
    Missing,
    VerificationPending,
}

#[derive(Clone, Copy, Debug, Eq, PartialOrd, Ord, PartialEq)]
pub enum OrdinaryQueueDisposition {
    InQueue,
    ExitedAfterCompleteAcceptOrTransfer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttentionInputError {
    RiskExitRequiresAcceptOrTransfer,
    RiskExitProofIncomplete,
    OpenIssueCannotCarryVerificationState,
    ResolvedIssueRequiresResolutionEvidence,
    ClosedIssueCannotRequireAttention,
    FailedVerificationRequiresReopenGuidance,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompleteRiskExitProof {
    response: crate::work_management::RiskResponseType,
    owner: StakeholderId,
    rationale: RiskRationale,
    residual_exposure: ResidualExposure,
    review_at: UtcTimestamp,
}

impl CompleteRiskExitProof {
    pub fn new(
        response: crate::work_management::RiskResponseType,
        owner: Option<StakeholderId>,
        rationale: Option<RiskRationale>,
        residual_exposure: Option<ResidualExposure>,
        review_at: Option<UtcTimestamp>,
    ) -> Result<Self, AttentionInputError> {
        if !matches!(
            response,
            crate::work_management::RiskResponseType::Accept
                | crate::work_management::RiskResponseType::Transfer
        ) {
            return Err(AttentionInputError::RiskExitRequiresAcceptOrTransfer);
        }
        Ok(Self {
            response,
            owner: owner.ok_or(AttentionInputError::RiskExitProofIncomplete)?,
            rationale: rationale.ok_or(AttentionInputError::RiskExitProofIncomplete)?,
            residual_exposure: residual_exposure
                .ok_or(AttentionInputError::RiskExitProofIncomplete)?,
            review_at: review_at.ok_or(AttentionInputError::RiskExitProofIncomplete)?,
        })
    }

    pub const fn response(&self) -> crate::work_management::RiskResponseType {
        self.response
    }
    pub fn owner(&self) -> &StakeholderId {
        &self.owner
    }
    pub fn rationale(&self) -> &RiskRationale {
        &self.rationale
    }
    pub fn residual_exposure(&self) -> &ResidualExposure {
        &self.residual_exposure
    }
    pub const fn review_at(&self) -> UtcTimestamp {
        self.review_at
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IssueEvidenceAttentionState {
    None,
    ResolutionEvidenceMissing,
    VerificationEvidenceMissing,
    VerificationPending,
    FailedVerification,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttentionMetadata {
    pub classification: DataClassification,
    pub freshness: Freshness,
    pub degraded: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttentionFlag {
    pub target: AttentionTarget,
    pub reason: AttentionReason,
    pub metadata: AttentionMetadata,
    pub explanation: &'static str,
    pub failed_verification_guidance: Option<FailedVerificationReopenGuidance>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AttentionThresholds {
    pub decision_approaching_deadline_millis: i64,
    pub action_at_risk_millis: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionRequestAttentionInput {
    pub id: ActionRequestId,
    pub state: ActionRequestState,
    pub intended_owner_present: bool,
    pub response_due_at: Option<UtcTimestamp>,
    pub needs_info: bool,
    pub metadata: AttentionMetadata,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionAttentionInput {
    pub id: ActionId,
    pub state: ActionState,
    pub due_at: UtcTimestamp,
    pub blocked: bool,
    pub at_risk: bool,
    pub evidence_state: EvidenceAttentionState,
    pub superseded_premise: bool,
    pub metadata: AttentionMetadata,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionRequestAttentionInput {
    pub id: DecisionRequestId,
    pub state: DecisionRequestState,
    pub decision_owner_present: bool,
    pub decision_deadline_at: Option<UtcTimestamp>,
    pub needs_info: bool,
    pub metadata: AttentionMetadata,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskAttentionInput {
    id: RiskId,
    state: RiskState,
    next_review_at: Option<UtcTimestamp>,
    exposure_increased: bool,
    evidence_stale: bool,
    control_invalid: bool,
    owner_present: bool,
    queue_disposition: OrdinaryQueueDisposition,
    exit_proof: Option<CompleteRiskExitProof>,
    metadata: AttentionMetadata,
}

impl RiskAttentionInput {
    #[allow(clippy::too_many_arguments)]
    pub const fn in_queue(
        id: RiskId,
        state: RiskState,
        next_review_at: Option<UtcTimestamp>,
        exposure_increased: bool,
        evidence_stale: bool,
        control_invalid: bool,
        owner_present: bool,
        metadata: AttentionMetadata,
    ) -> Self {
        Self {
            id,
            state,
            next_review_at,
            exposure_increased,
            evidence_stale,
            control_invalid,
            owner_present,
            queue_disposition: OrdinaryQueueDisposition::InQueue,
            exit_proof: None,
            metadata,
        }
    }

    pub fn exited_after_complete_accept_or_transfer(
        id: RiskId,
        state: RiskState,
        proof: CompleteRiskExitProof,
        exposure_increased: bool,
        evidence_stale: bool,
        control_invalid: bool,
        metadata: AttentionMetadata,
    ) -> Self {
        Self {
            id,
            state,
            next_review_at: Some(proof.review_at),
            exposure_increased,
            evidence_stale,
            control_invalid,
            owner_present: true,
            queue_disposition: OrdinaryQueueDisposition::ExitedAfterCompleteAcceptOrTransfer,
            exit_proof: Some(proof),
            metadata,
        }
    }

    pub fn complete_exit_proof(&self) -> Option<&CompleteRiskExitProof> {
        self.exit_proof.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FailedVerificationReopenGuidance {
    pub safe_explanation: crate::issues::IssueDetails,
    pub remediation: crate::issues::IssueDetails,
    pub reopen_rationale: crate::issues::IssueDetails,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum IssueAttentionSnapshot {
    Open {
        resolution_evidence_missing: bool,
    },
    Resolved {
        verification: ResolvedIssueVerification,
    },
    Closed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolvedIssueVerification {
    Complete,
    EvidenceMissing,
    Pending,
    Failed(FailedVerificationReopenGuidance),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssueAttentionInput {
    id: IssueId,
    snapshot: IssueAttentionSnapshot,
    resolution_due_at: Option<UtcTimestamp>,
    blocked: bool,
    recurrence_link_present: bool,
    metadata: AttentionMetadata,
}

impl IssueAttentionInput {
    pub const fn open(
        id: IssueId,
        resolution_due_at: Option<UtcTimestamp>,
        blocked: bool,
        resolution_evidence_missing: bool,
        recurrence_link_present: bool,
        metadata: AttentionMetadata,
    ) -> Self {
        Self {
            id,
            snapshot: IssueAttentionSnapshot::Open {
                resolution_evidence_missing,
            },
            resolution_due_at,
            blocked,
            recurrence_link_present,
            metadata,
        }
    }
    pub const fn resolved(
        id: IssueId,
        resolution_due_at: Option<UtcTimestamp>,
        blocked: bool,
        verification: ResolvedIssueVerification,
        recurrence_link_present: bool,
        metadata: AttentionMetadata,
    ) -> Self {
        Self {
            id,
            snapshot: IssueAttentionSnapshot::Resolved { verification },
            resolution_due_at,
            blocked,
            recurrence_link_present,
            metadata,
        }
    }
    pub const fn closed(id: IssueId, metadata: AttentionMetadata) -> Self {
        Self {
            id,
            snapshot: IssueAttentionSnapshot::Closed,
            resolution_due_at: None,
            blocked: false,
            recurrence_link_present: false,
            metadata,
        }
    }
    pub fn failed_verification_reopen_guidance(&self) -> Option<&FailedVerificationReopenGuidance> {
        match &self.snapshot {
            IssueAttentionSnapshot::Resolved {
                verification: ResolvedIssueVerification::Failed(guidance),
            } => Some(guidance),
            _ => None,
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        id: IssueId,
        state: IssueState,
        resolution_due_at: Option<UtcTimestamp>,
        blocked: bool,
        evidence_state: IssueEvidenceAttentionState,
        failed_guidance: Option<FailedVerificationReopenGuidance>,
        recurrence_link_present: bool,
        metadata: AttentionMetadata,
    ) -> Result<Self, AttentionInputError> {
        match (state, evidence_state) {
            (IssueState::Open, IssueEvidenceAttentionState::None) => Ok(Self::open(
                id,
                resolution_due_at,
                blocked,
                false,
                recurrence_link_present,
                metadata,
            )),
            (IssueState::Open, IssueEvidenceAttentionState::ResolutionEvidenceMissing) => {
                Ok(Self::open(
                    id,
                    resolution_due_at,
                    blocked,
                    true,
                    recurrence_link_present,
                    metadata,
                ))
            }
            (IssueState::Open, _) => {
                Err(AttentionInputError::OpenIssueCannotCarryVerificationState)
            }
            (IssueState::Resolved, IssueEvidenceAttentionState::None) => Ok(Self::resolved(
                id,
                resolution_due_at,
                blocked,
                ResolvedIssueVerification::Complete,
                recurrence_link_present,
                metadata,
            )),
            (IssueState::Resolved, IssueEvidenceAttentionState::ResolutionEvidenceMissing) => {
                Err(AttentionInputError::ResolvedIssueRequiresResolutionEvidence)
            }
            (IssueState::Resolved, IssueEvidenceAttentionState::VerificationEvidenceMissing) => {
                Ok(Self::resolved(
                    id,
                    resolution_due_at,
                    blocked,
                    ResolvedIssueVerification::EvidenceMissing,
                    recurrence_link_present,
                    metadata,
                ))
            }
            (IssueState::Resolved, IssueEvidenceAttentionState::VerificationPending) => {
                Ok(Self::resolved(
                    id,
                    resolution_due_at,
                    blocked,
                    ResolvedIssueVerification::Pending,
                    recurrence_link_present,
                    metadata,
                ))
            }
            (IssueState::Resolved, IssueEvidenceAttentionState::FailedVerification) => {
                Ok(Self::resolved(
                    id,
                    resolution_due_at,
                    blocked,
                    ResolvedIssueVerification::Failed(
                        failed_guidance
                            .ok_or(AttentionInputError::FailedVerificationRequiresReopenGuidance)?,
                    ),
                    recurrence_link_present,
                    metadata,
                ))
            }
            (IssueState::Closed, IssueEvidenceAttentionState::None)
                if resolution_due_at.is_none() && !blocked && !recurrence_link_present =>
            {
                Ok(Self::closed(id, metadata))
            }
            (IssueState::Closed, _) => Err(AttentionInputError::ClosedIssueCannotRequireAttention),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttentionInputs {
    pub as_of: UtcTimestamp,
    pub thresholds: AttentionThresholds,
    pub action_requests: Vec<ActionRequestAttentionInput>,
    pub actions: Vec<ActionAttentionInput>,
    pub decision_requests: Vec<DecisionRequestAttentionInput>,
    pub risks: Vec<RiskAttentionInput>,
    pub issues: Vec<IssueAttentionInput>,
}

impl Default for AttentionInputs {
    fn default() -> Self {
        Self {
            as_of: UtcTimestamp::from_unix_millis(0),
            thresholds: AttentionThresholds::default(),
            action_requests: vec![],
            actions: vec![],
            decision_requests: vec![],
            risks: vec![],
            issues: vec![],
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttentionResult {
    pub as_of: UtcTimestamp,
    pub flags: Vec<AttentionFlag>,
    pub risk_reentry_summaries: Vec<RiskReentrySummary>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskReentrySummary {
    pub target: AttentionTarget,
    pub reasons: Vec<AttentionReason>,
    pub explanation: &'static str,
}

pub fn derive_attention(input: &AttentionInputs) -> AttentionResult {
    let mut flags = Vec::new();
    let decision_window = i128::from(input.thresholds.decision_approaching_deadline_millis.max(0));
    let action_window = input
        .thresholds
        .action_at_risk_millis
        .map(|window| i128::from(window.max(0)));
    for item in &input.action_requests {
        if matches!(
            item.state,
            ActionRequestState::Accepted
                | ActionRequestState::Declined
                | ActionRequestState::Withdrawn
        ) {
            continue;
        }
        let target = AttentionTarget::ActionRequest(item.id.clone());
        push(
            &mut flags,
            &target,
            item.metadata,
            AttentionReason::ActionRequestNeedsInfo,
            item.needs_info,
            "additional information is required",
        );
        push(
            &mut flags,
            &target,
            item.metadata,
            AttentionReason::ActionRequestStale,
            matches!(item.metadata.freshness, Freshness::Stale),
            "the request information is stale",
        );
        push(
            &mut flags,
            &target,
            item.metadata,
            AttentionReason::ActionRequestMissingIntendedOwner,
            !item.intended_owner_present,
            "an intended owner is missing",
        );
        if let Some(deadline) = item.response_due_at {
            let overdue = deadline < input.as_of;
            push(
                &mut flags,
                &target,
                item.metadata,
                AttentionReason::ActionRequestResponseOverdue,
                overdue,
                "the response deadline has passed",
            );
            push(
                &mut flags,
                &target,
                item.metadata,
                AttentionReason::ActionRequestResponseDue,
                !overdue && deadline == input.as_of,
                "the response is due",
            );
        }
    }
    for item in &input.actions {
        if matches!(item.state, ActionState::Completed | ActionState::Cancelled) {
            continue;
        }
        let target = AttentionTarget::Action(item.id.clone());
        push(
            &mut flags,
            &target,
            item.metadata,
            AttentionReason::ActionBlocked,
            item.blocked,
            "the action is blocked",
        );
        push(
            &mut flags,
            &target,
            item.metadata,
            AttentionReason::ActionOverdue,
            item.due_at < input.as_of,
            "the action deadline has passed",
        );
        let at_risk = item.at_risk
            || action_window.is_some_and(|window| {
                let remaining =
                    i128::from(item.due_at.unix_millis()) - i128::from(input.as_of.unix_millis());
                remaining >= 0 && remaining <= window
            });
        push(
            &mut flags,
            &target,
            item.metadata,
            AttentionReason::ActionAtRisk,
            at_risk,
            "the action is at risk",
        );
        push(
            &mut flags,
            &target,
            item.metadata,
            AttentionReason::ActionNeedsEvidence,
            matches!(item.evidence_state, EvidenceAttentionState::Missing),
            "completion evidence is required",
        );
        push(
            &mut flags,
            &target,
            item.metadata,
            AttentionReason::ActionEvidenceVerificationPending,
            matches!(
                item.evidence_state,
                EvidenceAttentionState::VerificationPending
            ),
            "completion evidence is awaiting verification",
        );
        push(
            &mut flags,
            &target,
            item.metadata,
            AttentionReason::ActionSupersededPremise,
            item.superseded_premise,
            "the action premise was superseded",
        );
    }
    for item in &input.decision_requests {
        if matches!(
            item.state,
            DecisionRequestState::Resolved | DecisionRequestState::Withdrawn
        ) {
            continue;
        }
        let target = AttentionTarget::DecisionRequest(item.id.clone());
        push(
            &mut flags,
            &target,
            item.metadata,
            AttentionReason::DecisionRequestNeedsInfo,
            item.needs_info,
            "additional information is required",
        );
        push(
            &mut flags,
            &target,
            item.metadata,
            AttentionReason::DecisionRequestMissingDecisionOwner,
            !item.decision_owner_present,
            "a decision owner is missing",
        );
        push(
            &mut flags,
            &target,
            item.metadata,
            AttentionReason::DecisionRequestStale,
            matches!(item.metadata.freshness, Freshness::Stale),
            "the decision request information is stale",
        );
        if let Some(deadline) = item.decision_deadline_at {
            let remaining =
                i128::from(deadline.unix_millis()) - i128::from(input.as_of.unix_millis());
            push(
                &mut flags,
                &target,
                item.metadata,
                AttentionReason::DecisionRequestOverdue,
                remaining < 0,
                "the decision deadline has passed",
            );
            push(
                &mut flags,
                &target,
                item.metadata,
                AttentionReason::DecisionRequestApproachingDeadline,
                remaining >= 0 && remaining <= decision_window,
                "the decision deadline is approaching",
            );
        }
    }
    for item in &input.risks {
        if matches!(item.state, RiskState::Occurred | RiskState::Closed) {
            continue;
        }
        let target = AttentionTarget::Risk(item.id.clone());
        push(
            &mut flags,
            &target,
            item.metadata,
            AttentionReason::RiskReviewDue,
            item.next_review_at.is_some_and(|at| at <= input.as_of),
            "the risk review is due",
        );
        push(
            &mut flags,
            &target,
            item.metadata,
            AttentionReason::RiskExposureIncreased,
            item.exposure_increased,
            "risk exposure increased",
        );
        push(
            &mut flags,
            &target,
            item.metadata,
            AttentionReason::RiskEvidenceStale,
            item.evidence_stale,
            "risk evidence is stale",
        );
        push(
            &mut flags,
            &target,
            item.metadata,
            AttentionReason::RiskControlInvalid,
            item.control_invalid,
            "the risk control is invalid",
        );
        push(
            &mut flags,
            &target,
            item.metadata,
            AttentionReason::RiskMissingOwner,
            !item.owner_present,
            "a risk owner is missing",
        );
    }
    for item in &input.issues {
        if matches!(item.snapshot, IssueAttentionSnapshot::Closed) {
            continue;
        }
        let target = AttentionTarget::Issue(item.id.clone());
        push(
            &mut flags,
            &target,
            item.metadata,
            AttentionReason::IssueBlocked,
            item.blocked,
            "the issue is blocked",
        );
        if let Some(deadline) = item.resolution_due_at {
            let overdue = deadline < input.as_of;
            push(
                &mut flags,
                &target,
                item.metadata,
                AttentionReason::IssueResolutionOverdue,
                overdue,
                "the issue resolution deadline has passed",
            );
            push(
                &mut flags,
                &target,
                item.metadata,
                AttentionReason::IssueResolutionDue,
                !overdue && deadline == input.as_of,
                "the issue resolution is due",
            );
        }
        let issue_evidence_missing = matches!(
            item.snapshot,
            IssueAttentionSnapshot::Open {
                resolution_evidence_missing: true
            }
        );
        let verification_missing = matches!(
            item.snapshot,
            IssueAttentionSnapshot::Resolved {
                verification: ResolvedIssueVerification::EvidenceMissing
            }
        );
        let verification_pending = matches!(
            item.snapshot,
            IssueAttentionSnapshot::Resolved {
                verification: ResolvedIssueVerification::Pending
            }
        );
        push(
            &mut flags,
            &target,
            item.metadata,
            AttentionReason::IssueNeedsEvidence,
            issue_evidence_missing || verification_missing || verification_pending,
            if issue_evidence_missing {
                "issue resolution evidence is required"
            } else if verification_missing {
                "issue closure verification evidence is required"
            } else {
                "issue verification evidence is pending"
            },
        );
        if let IssueAttentionSnapshot::Resolved {
            verification: ResolvedIssueVerification::Failed(guidance),
        } = &item.snapshot
        {
            flags.push(AttentionFlag {
                target: target.clone(),
                reason: AttentionReason::IssueNeedsEvidence,
                metadata: item.metadata,
                explanation: "issue verification failed; reasoned reopening is required",
                failed_verification_guidance: Some(guidance.clone()),
            });
        }
        push(
            &mut flags,
            &target,
            item.metadata,
            AttentionReason::IssueStale,
            matches!(item.metadata.freshness, Freshness::Stale),
            "issue information is stale",
        );
        push(
            &mut flags,
            &target,
            item.metadata,
            AttentionReason::IssueRecurrence,
            item.recurrence_link_present,
            "the issue has a recurrence link",
        );
    }
    flags.sort_by(|a, b| (&a.target, a.reason).cmp(&(&b.target, b.reason)));
    let mut risk_reentry_summaries = Vec::new();
    for item in &input.risks {
        let target = AttentionTarget::Risk(item.id.clone());
        let mut reasons: Vec<_> = flags
            .iter()
            .filter(|flag| flag.target == target)
            .map(|flag| flag.reason)
            .collect();
        let exited = matches!(
            item.queue_disposition,
            OrdinaryQueueDisposition::ExitedAfterCompleteAcceptOrTransfer
        );
        reasons.retain(|reason| {
            matches!(
                reason,
                AttentionReason::RiskReviewDue
                    | AttentionReason::RiskExposureIncreased
                    | AttentionReason::RiskEvidenceStale
                    | AttentionReason::RiskControlInvalid
            )
        });
        if exited && reasons.len() >= 2 {
            risk_reentry_summaries.push(RiskReentrySummary {
                target,
                reasons,
                explanation: "multiple active risk conditions require combined review",
            });
        }
    }
    risk_reentry_summaries.sort_by(|left, right| left.target.cmp(&right.target));
    AttentionResult {
        as_of: input.as_of,
        flags,
        risk_reentry_summaries,
    }
}

fn push(
    flags: &mut Vec<AttentionFlag>,
    target: &AttentionTarget,
    metadata: AttentionMetadata,
    reason: AttentionReason,
    enabled: bool,
    explanation: &'static str,
) {
    if enabled {
        flags.push(AttentionFlag {
            target: target.clone(),
            reason,
            metadata,
            explanation,
            failed_verification_guidance: None,
        });
    }
}
