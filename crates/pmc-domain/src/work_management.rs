//! Closed, typed Work Management lifecycle and H2a preparation primitives.

#[cfg(test)]
use std::collections::HashMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use sha2::{Digest, Sha256};

use crate::audit::{AuditActor, AuditEvent};
use crate::classification::DataClassification;
use crate::identity::{
    ActionId, ActionRequestId, AggregateVersion, ApprovalReceiptId, DecisionId, DecisionRequestId,
    EvidenceReferenceId, IdempotencyId, InitiativeId, IssueId, KpiId, KpiObservationId,
    MilestoneId, PortfolioId, PreparedIntentId, ProductId, ProjectId, RiskId, RoadmapId,
    StakeholderId,
};
#[cfg(test)]
use crate::time::Clock;
use crate::time::UtcTimestamp;
use crate::value::{BoundedText, DomainValueError, ValueErrorKind};

pub const SUPPORTED_WORK_MANAGEMENT_H2A_CONTRACT_VERSION: u16 = 1;
pub const WORK_MANAGEMENT_H2A_TTL_MILLIS: i64 = 300_000;

macro_rules! persisted_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum $name { $($variant),+ }

        impl $name {
            #[must_use]
            pub const fn as_persisted(self) -> &'static str {
                match self { $(Self::$variant => $value),+ }
            }

            pub fn from_persisted(value: &str) -> Result<Self, DomainValueError> {
                match value {
                    $($value => Ok(Self::$variant),)+
                    _ => Err(DomainValueError::new(ValueErrorKind::UnknownPersistedValue)),
                }
            }
        }
    };
}

persisted_enum!(ActionRequestState {
    Draft => "draft", Open => "open", Accepted => "accepted", Declined => "declined",
    Withdrawn => "withdrawn"
});
persisted_enum!(ActionState {
    Open => "open", InProgress => "in_progress", Completed => "completed", Cancelled => "cancelled"
});
persisted_enum!(DecisionRequestState {
    Draft => "draft", Open => "open", Resolved => "resolved", Withdrawn => "withdrawn"
});
persisted_enum!(DecisionState { Effective => "effective", Superseded => "superseded" });
persisted_enum!(RiskState { Open => "open", Occurred => "occurred", Closed => "closed" });
persisted_enum!(IssueState { Open => "open", Resolved => "resolved", Closed => "closed" });
persisted_enum!(IssueResolutionType {
    Resolved => "resolved", Workaround => "workaround", AcceptedImpact => "accepted_impact"
});
persisted_enum!(RiskResponseType {
    Mitigate => "mitigate", Accept => "accept", Transfer => "transfer", Avoid => "avoid"
});

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IntegrityDigest(BoundedText<64>);

impl IntegrityDigest {
    pub fn parse(value: impl Into<String>) -> Result<Self, DomainValueError> {
        let value = value.into();
        if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(DomainValueError::new(ValueErrorKind::InvalidCharacter));
        }
        Ok(Self(BoundedText::parse(value.to_ascii_lowercase())?))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvidenceVerification {
    Verified {
        verified_at: UtcTimestamp,
        integrity_digest: IntegrityDigest,
    },
    DegradedLastVerified {
        last_verified_at: UtcTimestamp,
        integrity_digest: IntegrityDigest,
    },
    Unverified,
    IntegrityMismatch,
    /// The file was read and hashed, but the reference carries no pinned
    /// fingerprint, so there is nothing to compare the hash against.
    ///
    /// This is a distinct state on purpose. Reporting it as `Verified`
    /// claimed an integrity check that never happened: an unpinned file could
    /// change arbitrarily and every observation would still say verified.
    /// Reporting it as `Unverified` would conflate "could not be read" with
    /// "read fine, no identity pin", and would drop the observed digest and
    /// time, so a later change to the file left no trace of re-observation.
    ///
    /// It never satisfies an evidence gate on its own. With a recorded human
    /// judgment it yields `VerificationPending`, the same path a degraded
    /// verification takes, so an unpinned reference is a visible limitation
    /// rather than a dead end. The way out is pinning.
    ObservedUnpinned {
        observed_at: UtcTimestamp,
        integrity_digest: IntegrityDigest,
    },
}

impl EvidenceVerification {
    /// Rebuilds a verification state from the three columns the Ledger
    /// persists it as.
    ///
    /// The one decoder for both the write path's replay reads and the
    /// composition read surface. Two decoders for the same three columns
    /// would drift, and a drift here means one surface calling Evidence
    /// verified that another calls unverified.
    ///
    /// `Verified` and `DegradedLastVerified` require both a timestamp and a
    /// digest; their absence is a storage error, not a weaker state.
    pub fn from_persisted_parts(
        kind: &str,
        last_verified_at: Option<UtcTimestamp>,
        integrity_digest: Option<IntegrityDigest>,
    ) -> Result<Self, DomainValueError> {
        let missing = || DomainValueError::new(ValueErrorKind::UnknownPersistedValue);
        match kind {
            "verified" => Ok(Self::Verified {
                verified_at: last_verified_at.ok_or_else(missing)?,
                integrity_digest: integrity_digest.ok_or_else(missing)?,
            }),
            "degraded_last_verified" => Ok(Self::DegradedLastVerified {
                last_verified_at: last_verified_at.ok_or_else(missing)?,
                integrity_digest: integrity_digest.ok_or_else(missing)?,
            }),
            "unverified" => Ok(Self::Unverified),
            "integrity_mismatch" => Ok(Self::IntegrityMismatch),
            "observed_unpinned" => Ok(Self::ObservedUnpinned {
                observed_at: last_verified_at.ok_or_else(missing)?,
                integrity_digest: integrity_digest.ok_or_else(missing)?,
            }),
            _ => Err(missing()),
        }
    }

    /// The persisted name of the state, without its payload.
    #[must_use]
    pub const fn kind_as_persisted(&self) -> &'static str {
        match self {
            Self::Verified { .. } => "verified",
            Self::DegradedLastVerified { .. } => "degraded_last_verified",
            Self::Unverified => "unverified",
            Self::IntegrityMismatch => "integrity_mismatch",
            Self::ObservedUnpinned { .. } => "observed_unpinned",
        }
    }
}

/// What a support witness records about one Evidence reference at the
/// instant it was prepared. `source_version` is the reference's aggregate
/// version at that instant (DG3: the exact preview and hash bind source
/// revisions), so any change to the reference between prepare and execute
/// -- a relocation, a re-observation, a pin -- changes the digest even when
/// the verification tuple reads the same.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceReferenceMetadata {
    id: EvidenceReferenceId,
    source_version: AggregateVersion,
    classification: DataClassification,
    role: EvidenceRole,
    verification: EvidenceVerification,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceRole {
    ActionCompletion,
    DecisionResolution,
    IssueResolution,
    IssueClosureVerification,
    IssueFailedVerification,
}

impl EvidenceRole {
    /// The persisted name. One table, shared with the support digest via
    /// [`evidence_role_name`], so a read surface and the digest can never
    /// disagree about what a role is called.
    #[must_use]
    pub const fn as_persisted(self) -> &'static str {
        evidence_role_name(self)
    }

    pub fn from_persisted(value: &str) -> Result<Self, DomainValueError> {
        match value {
            "action_completion" => Ok(Self::ActionCompletion),
            "decision_resolution" => Ok(Self::DecisionResolution),
            "issue_resolution" => Ok(Self::IssueResolution),
            "issue_closure_verification" => Ok(Self::IssueClosureVerification),
            "issue_failed_verification" => Ok(Self::IssueFailedVerification),
            _ => Err(DomainValueError::new(ValueErrorKind::UnknownPersistedValue)),
        }
    }
}

impl EvidenceReferenceMetadata {
    #[must_use]
    pub const fn new(
        id: EvidenceReferenceId,
        source_version: AggregateVersion,
        classification: DataClassification,
        role: EvidenceRole,
        verification: EvidenceVerification,
    ) -> Self {
        Self {
            id,
            source_version,
            classification,
            role,
            verification,
        }
    }

    #[must_use]
    pub const fn id(&self) -> &EvidenceReferenceId {
        &self.id
    }
    /// The reference's aggregate version when this witness was prepared.
    #[must_use]
    pub const fn source_version(&self) -> AggregateVersion {
        self.source_version
    }
    #[must_use]
    pub const fn classification(&self) -> DataClassification {
        self.classification
    }
    #[must_use]
    pub const fn role(&self) -> EvidenceRole {
        self.role
    }
    #[must_use]
    pub const fn verification(&self) -> &EvidenceVerification {
        &self.verification
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HumanJudgmentDisposition {
    ProceedWithDocumentedRationale,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HumanJudgment {
    disposition: HumanJudgmentDisposition,
    rationale: BoundedText<2_000>,
    classification: DataClassification,
}

impl HumanJudgment {
    pub fn new(
        disposition: HumanJudgmentDisposition,
        rationale: impl Into<String>,
        classification: DataClassification,
    ) -> Result<Self, DomainValueError> {
        Ok(Self {
            disposition,
            rationale: BoundedText::parse(rationale.into())?,
            classification,
        })
    }

    #[must_use]
    pub const fn actor(&self) -> AuditActor {
        AuditActor::HeadOfProducts
    }
    #[must_use]
    pub const fn disposition(&self) -> HumanJudgmentDisposition {
        self.disposition
    }
    #[must_use]
    pub fn rationale(&self) -> &str {
        self.rationale.as_str()
    }
    #[must_use]
    pub const fn classification(&self) -> DataClassification {
        self.classification
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupportDisposition {
    EvidenceSatisfied,
    JudgmentSatisfied,
    VerificationPending,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SupportRequirement {
    EvidenceRequired,
    EvidenceOrJudgment,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SupportWitness {
    requirement: SupportRequirement,
    disposition: SupportDisposition,
    evidence: Vec<EvidenceReferenceMetadata>,
    judgments: Vec<HumanJudgment>,
    classification: DataClassification,
}

impl SupportWitness {
    #[must_use]
    pub const fn disposition(&self) -> SupportDisposition {
        self.disposition
    }
    #[must_use]
    pub const fn classification(&self) -> DataClassification {
        self.classification
    }
    #[must_use]
    pub fn evidence(&self) -> &[EvidenceReferenceMetadata] {
        &self.evidence
    }
    #[must_use]
    pub fn judgments(&self) -> &[HumanJudgment] {
        &self.judgments
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupportGateError {
    Empty,
    EvidenceRequiredNotSatisfied,
}

impl Display for SupportGateError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "evidence or human judgment is required",
            Self::EvidenceRequiredNotSatisfied => "verified evidence is required",
        })
    }
}

impl Error for SupportGateError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceOrJudgment {
    evidence: Vec<EvidenceReferenceMetadata>,
    judgments: Vec<HumanJudgment>,
}

impl EvidenceOrJudgment {
    pub fn new(
        evidence: Vec<EvidenceReferenceMetadata>,
        judgments: Vec<HumanJudgment>,
    ) -> Result<Self, SupportGateError> {
        if evidence.is_empty() && judgments.is_empty() {
            return Err(SupportGateError::Empty);
        }
        Ok(Self {
            evidence,
            judgments,
        })
    }

    pub fn evaluate_evidence_required(&self) -> Result<SupportWitness, SupportGateError> {
        if self.evidence.iter().any(|item| {
            matches!(
                item.verification,
                EvidenceVerification::Unverified | EvidenceVerification::IntegrityMismatch
            )
        }) {
            return Err(SupportGateError::EvidenceRequiredNotSatisfied);
        }
        let has_degraded = self.evidence.iter().any(|item| {
            matches!(
                item.verification,
                EvidenceVerification::DegradedLastVerified { .. }
                    | EvidenceVerification::ObservedUnpinned { .. }
            )
        });
        if has_degraded {
            if self.judgments.is_empty() {
                return Err(SupportGateError::EvidenceRequiredNotSatisfied);
            }
            return Ok(self.witness(
                SupportRequirement::EvidenceRequired,
                SupportDisposition::VerificationPending,
            ));
        }
        if self
            .evidence
            .iter()
            .any(|item| matches!(item.verification, EvidenceVerification::Verified { .. }))
        {
            return Ok(self.witness(
                SupportRequirement::EvidenceRequired,
                SupportDisposition::EvidenceSatisfied,
            ));
        }
        Err(SupportGateError::EvidenceRequiredNotSatisfied)
    }

    pub fn evaluate_evidence_or_judgment(&self) -> Result<SupportWitness, SupportGateError> {
        if self.evidence.iter().any(|item| {
            matches!(
                item.verification,
                EvidenceVerification::Unverified | EvidenceVerification::IntegrityMismatch
            )
        }) {
            return Err(SupportGateError::EvidenceRequiredNotSatisfied);
        }
        let has_degraded = self.evidence.iter().any(|item| {
            matches!(
                item.verification,
                EvidenceVerification::DegradedLastVerified { .. }
                    | EvidenceVerification::ObservedUnpinned { .. }
            )
        });
        if has_degraded {
            if self.judgments.is_empty() {
                return Err(SupportGateError::EvidenceRequiredNotSatisfied);
            }
            Ok(self.witness(
                SupportRequirement::EvidenceOrJudgment,
                SupportDisposition::VerificationPending,
            ))
        } else if self
            .evidence
            .iter()
            .any(|item| matches!(item.verification, EvidenceVerification::Verified { .. }))
        {
            Ok(self.witness(
                SupportRequirement::EvidenceOrJudgment,
                SupportDisposition::EvidenceSatisfied,
            ))
        } else if !self.judgments.is_empty() {
            Ok(self.witness(
                SupportRequirement::EvidenceOrJudgment,
                SupportDisposition::JudgmentSatisfied,
            ))
        } else {
            Err(SupportGateError::EvidenceRequiredNotSatisfied)
        }
    }

    fn witness(
        &self,
        requirement: SupportRequirement,
        disposition: SupportDisposition,
    ) -> SupportWitness {
        let mut classifications = self
            .evidence
            .iter()
            .map(EvidenceReferenceMetadata::classification)
            .chain(self.judgments.iter().map(HumanJudgment::classification));
        let classification = classifications
            .next()
            .map_or(DataClassification::Unclassified, |first| {
                classifications.fold(first, DataClassification::combine)
            });
        SupportWitness {
            requirement,
            disposition,
            evidence: self.evidence.clone(),
            judgments: self.judgments.clone(),
            classification,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkManagementH2aIntentKind {
    AcceptActionRequest,
    ResolveDecisionRequest,
    CompleteAction,
    CancelAction,
    ReopenAction,
    SupersedeDecision,
    RecordRiskOccurrence,
    CloseRisk,
    ResolveIssue,
    CloseIssue,
    ReopenIssue,
    LowerPortfolioClassification,
    LowerProductClassification,
    LowerRoadmapClassification,
    LowerKpiClassification,
    LowerKpiObservationClassification,
    LowerActionClassification,
    LowerDecisionClassification,
    LowerRiskClassification,
    LowerIssueClassification,
    LowerInitiativeClassification,
    LowerProjectClassification,
    LowerMilestoneClassification,
}

impl WorkManagementH2aIntentKind {
    pub const ALL: [Self; 23] = [
        Self::AcceptActionRequest,
        Self::ResolveDecisionRequest,
        Self::CompleteAction,
        Self::CancelAction,
        Self::ReopenAction,
        Self::SupersedeDecision,
        Self::RecordRiskOccurrence,
        Self::CloseRisk,
        Self::ResolveIssue,
        Self::CloseIssue,
        Self::ReopenIssue,
        Self::LowerPortfolioClassification,
        Self::LowerProductClassification,
        Self::LowerRoadmapClassification,
        Self::LowerKpiClassification,
        Self::LowerKpiObservationClassification,
        Self::LowerActionClassification,
        Self::LowerDecisionClassification,
        Self::LowerRiskClassification,
        Self::LowerIssueClassification,
        Self::LowerInitiativeClassification,
        Self::LowerProjectClassification,
        Self::LowerMilestoneClassification,
    ];

    #[must_use]
    pub const fn as_persisted(self) -> &'static str {
        match self {
            Self::AcceptActionRequest => "accept_action_request",
            Self::ResolveDecisionRequest => "resolve_decision_request",
            Self::CompleteAction => "complete_action",
            Self::CancelAction => "cancel_action",
            Self::ReopenAction => "reopen_action",
            Self::SupersedeDecision => "supersede_decision",
            Self::RecordRiskOccurrence => "record_risk_occurrence",
            Self::CloseRisk => "close_risk",
            Self::ResolveIssue => "resolve_issue",
            Self::CloseIssue => "close_issue",
            Self::ReopenIssue => "reopen_issue",
            Self::LowerPortfolioClassification => "lower_portfolio_classification",
            Self::LowerProductClassification => "lower_product_classification",
            Self::LowerRoadmapClassification => "lower_roadmap_classification",
            Self::LowerKpiClassification => "lower_kpi_classification",
            Self::LowerKpiObservationClassification => "lower_kpi_observation_classification",
            Self::LowerActionClassification => "lower_action_classification",
            Self::LowerDecisionClassification => "lower_decision_classification",
            Self::LowerRiskClassification => "lower_risk_classification",
            Self::LowerIssueClassification => "lower_issue_classification",
            Self::LowerInitiativeClassification => "lower_initiative_classification",
            Self::LowerProjectClassification => "lower_project_classification",
            Self::LowerMilestoneClassification => "lower_milestone_classification",
        }
    }

    pub fn from_persisted(value: &str) -> Result<Self, DomainValueError> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_persisted() == value)
            .ok_or_else(|| DomainValueError::new(ValueErrorKind::UnknownPersistedValue))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkManagementOperation {
    AcceptActionRequest {
        request_id: ActionRequestId,
        request_version: AggregateVersion,
        action_id: ActionId,
        action_classification: DataClassification,
        action_subject: BoundedText<240>,
        commitment_details: BoundedText<2_000>,
        intended_owner: StakeholderId,
        intended_due_at: UtcTimestamp,
    },
    ResolveDecisionRequest {
        request_id: DecisionRequestId,
        request_version: AggregateVersion,
        decision_id: DecisionId,
        decision_classification: DataClassification,
        statement: BoundedText<4_000>,
        rationale: BoundedText<4_000>,
        impact: BoundedText<4_000>,
        decision_owner: StakeholderId,
        decided_at: UtcTimestamp,
        resulting_action_requests: Vec<DecisionResultingActionRequest>,
    },
    CompleteAction {
        action_id: ActionId,
        action_version: AggregateVersion,
    },
    CancelAction {
        action_id: ActionId,
        action_version: AggregateVersion,
        reason: WorkManagementRationale,
        evidence_classifications: Vec<EvidenceClassificationBinding>,
    },
    ReopenAction {
        action_id: ActionId,
        action_version: AggregateVersion,
        mode: ActionReopenMode,
        reason: WorkManagementRationale,
        evidence_classifications: Vec<EvidenceClassificationBinding>,
    },
    SupersedeDecision {
        decision_id: DecisionId,
        decision_version: AggregateVersion,
        replacement_decision_id: DecisionId,
        replacement_decision_version: AggregateVersion,
        replacement_decision_classification: DataClassification,
        replacement_statement: BoundedText<4_000>,
        replacement_rationale: BoundedText<4_000>,
        replacement_impact: BoundedText<4_000>,
        replacement_owner: StakeholderId,
        replacement_decided_at: UtcTimestamp,
        resulting_action_requests: Vec<DecisionResultingActionRequest>,
        incomplete_downstream: Vec<IncompleteDownstreamWork>,
    },
    RecordRiskOccurrence {
        risk_id: RiskId,
        risk_version: AggregateVersion,
        issue_id: IssueId,
        issue_classification: DataClassification,
    },
    CloseRisk {
        risk_id: RiskId,
        risk_version: AggregateVersion,
        rationale: WorkManagementRationale,
    },
    ResolveIssue {
        issue_id: IssueId,
        issue_version: AggregateVersion,
        resolution_type: IssueResolutionType,
        rationale: WorkManagementRationale,
    },
    CloseIssue {
        issue_id: IssueId,
        issue_version: AggregateVersion,
    },
    ReopenIssue {
        issue_id: IssueId,
        issue_version: AggregateVersion,
        rationale: WorkManagementRationale,
    },
    LowerPortfolioClassification {
        portfolio_id: PortfolioId,
        portfolio_version: AggregateVersion,
        current_classification: DataClassification,
        proposed_classification: DataClassification,
        rationale: WorkManagementRationale,
    },
    LowerProductClassification {
        product_id: ProductId,
        product_version: AggregateVersion,
        current_classification: DataClassification,
        proposed_classification: DataClassification,
        rationale: WorkManagementRationale,
    },
    LowerRoadmapClassification {
        roadmap_id: RoadmapId,
        roadmap_version: AggregateVersion,
        current_classification: DataClassification,
        proposed_classification: DataClassification,
        rationale: WorkManagementRationale,
    },
    LowerKpiClassification {
        kpi_id: KpiId,
        kpi_version: AggregateVersion,
        current_classification: DataClassification,
        proposed_classification: DataClassification,
        rationale: WorkManagementRationale,
    },
    LowerKpiObservationClassification {
        observation_id: KpiObservationId,
        observation_version: AggregateVersion,
        current_classification: DataClassification,
        proposed_classification: DataClassification,
        rationale: WorkManagementRationale,
    },
    LowerActionClassification {
        action_id: ActionId,
        action_version: AggregateVersion,
        current_classification: DataClassification,
        proposed_classification: DataClassification,
        rationale: WorkManagementRationale,
    },
    LowerDecisionClassification {
        decision_id: DecisionId,
        decision_version: AggregateVersion,
        current_classification: DataClassification,
        proposed_classification: DataClassification,
        rationale: WorkManagementRationale,
    },
    LowerRiskClassification {
        risk_id: RiskId,
        risk_version: AggregateVersion,
        current_classification: DataClassification,
        proposed_classification: DataClassification,
        rationale: WorkManagementRationale,
    },
    LowerIssueClassification {
        issue_id: IssueId,
        issue_version: AggregateVersion,
        current_classification: DataClassification,
        proposed_classification: DataClassification,
        rationale: WorkManagementRationale,
    },
    LowerInitiativeClassification {
        initiative_id: InitiativeId,
        initiative_version: AggregateVersion,
        current_classification: DataClassification,
        proposed_classification: DataClassification,
        rationale: WorkManagementRationale,
    },
    LowerProjectClassification {
        project_id: ProjectId,
        project_version: AggregateVersion,
        current_classification: DataClassification,
        proposed_classification: DataClassification,
        rationale: WorkManagementRationale,
    },
    LowerMilestoneClassification {
        milestone_id: MilestoneId,
        milestone_version: AggregateVersion,
        current_classification: DataClassification,
        proposed_classification: DataClassification,
        rationale: WorkManagementRationale,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionResultingActionRequest {
    pub id: ActionRequestId,
    pub subject: BoundedText<240>,
    pub details: BoundedText<2_000>,
    pub intended_owner: StakeholderId,
    pub due_at: UtcTimestamp,
    pub classification: DataClassification,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceClassificationBinding {
    evidence_id: EvidenceReferenceId,
    classification: DataClassification,
}

impl EvidenceClassificationBinding {
    #[must_use]
    pub const fn new(evidence_id: EvidenceReferenceId, classification: DataClassification) -> Self {
        Self {
            evidence_id,
            classification,
        }
    }

    #[must_use]
    pub const fn evidence_id(&self) -> &EvidenceReferenceId {
        &self.evidence_id
    }

    #[must_use]
    pub const fn classification(&self) -> DataClassification {
        self.classification
    }
}

pub type WorkManagementRationale = BoundedText<2_000>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionReopenMode {
    ReopenCompleted,
    RestartCancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IncompleteDownstreamWork {
    ActionRequest(ActionRequestId, AggregateVersion, DataClassification),
    Action(ActionId, AggregateVersion, DataClassification),
}

fn downstream_id(value: &IncompleteDownstreamWork) -> &str {
    match value {
        IncompleteDownstreamWork::ActionRequest(id, _, _) => id.as_str(),
        IncompleteDownstreamWork::Action(id, _, _) => id.as_str(),
    }
}

const fn downstream_kind(value: &IncompleteDownstreamWork) -> u8 {
    match value {
        IncompleteDownstreamWork::ActionRequest(_, _, _) => 0,
        IncompleteDownstreamWork::Action(_, _, _) => 1,
    }
}

fn same_downstream_identity(
    left: &IncompleteDownstreamWork,
    right: &IncompleteDownstreamWork,
) -> bool {
    downstream_kind(left) == downstream_kind(right) && downstream_id(left) == downstream_id(right)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkManagementTarget {
    ActionRequest(ActionRequestId, AggregateVersion),
    Action(ActionId, AggregateVersion),
    DecisionRequest(DecisionRequestId, AggregateVersion),
    Decision(DecisionId, AggregateVersion),
    Risk(RiskId, AggregateVersion),
    Issue(IssueId, AggregateVersion),
    Portfolio(PortfolioId, AggregateVersion),
    Product(ProductId, AggregateVersion),
    Roadmap(RoadmapId, AggregateVersion),
    Kpi(KpiId, AggregateVersion),
    KpiObservation(KpiObservationId, AggregateVersion),
    Initiative(InitiativeId, AggregateVersion),
    Project(ProjectId, AggregateVersion),
    Milestone(MilestoneId, AggregateVersion),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkManagementEffect {
    AcceptActionRequest(ActionRequestId),
    CreateAction(ActionId),
    LinkActionRequestToAction(ActionRequestId, ActionId),
    ResolveDecisionRequest(DecisionRequestId),
    CreateDecision(DecisionId),
    LinkDecisionRequestToDecision(DecisionRequestId, DecisionId),
    CreateResultingActionRequest(ActionRequestId),
    LinkDecisionToActionRequest(DecisionId, ActionRequestId),
    CompleteAction(ActionId),
    CancelAction(ActionId),
    ReopenAction(ActionId),
    SupersedeDecision(DecisionId),
    LinkReplacementDecision(DecisionId, DecisionId),
    FlagSupersededPremiseActionRequest(ActionRequestId),
    FlagSupersededPremiseAction(ActionId),
    RecordRiskOccurrence(RiskId),
    CreateIssue(IssueId),
    LinkRiskToIssue(RiskId, IssueId),
    CloseRisk(RiskId),
    ResolveIssue(IssueId),
    CloseIssue(IssueId),
    ReopenIssue(IssueId),
    LowerPortfolioClassification(PortfolioId),
    LowerProductClassification(ProductId),
    LowerRoadmapClassification(RoadmapId),
    LowerKpiClassification(KpiId),
    LowerKpiObservationClassification(KpiObservationId),
    LowerActionClassification(ActionId),
    LowerDecisionClassification(DecisionId),
    LowerRiskClassification(RiskId),
    LowerIssueClassification(IssueId),
    LowerInitiativeClassification(InitiativeId),
    LowerProjectClassification(ProjectId),
    LowerMilestoneClassification(MilestoneId),
}

impl WorkManagementOperation {
    const fn kind(&self) -> WorkManagementH2aIntentKind {
        match self {
            Self::AcceptActionRequest { .. } => WorkManagementH2aIntentKind::AcceptActionRequest,
            Self::ResolveDecisionRequest { .. } => {
                WorkManagementH2aIntentKind::ResolveDecisionRequest
            }
            Self::CompleteAction { .. } => WorkManagementH2aIntentKind::CompleteAction,
            Self::CancelAction { .. } => WorkManagementH2aIntentKind::CancelAction,
            Self::ReopenAction { .. } => WorkManagementH2aIntentKind::ReopenAction,
            Self::SupersedeDecision { .. } => WorkManagementH2aIntentKind::SupersedeDecision,
            Self::RecordRiskOccurrence { .. } => WorkManagementH2aIntentKind::RecordRiskOccurrence,
            Self::CloseRisk { .. } => WorkManagementH2aIntentKind::CloseRisk,
            Self::ResolveIssue { .. } => WorkManagementH2aIntentKind::ResolveIssue,
            Self::CloseIssue { .. } => WorkManagementH2aIntentKind::CloseIssue,
            Self::ReopenIssue { .. } => WorkManagementH2aIntentKind::ReopenIssue,
            Self::LowerPortfolioClassification { .. } => {
                WorkManagementH2aIntentKind::LowerPortfolioClassification
            }
            Self::LowerProductClassification { .. } => {
                WorkManagementH2aIntentKind::LowerProductClassification
            }
            Self::LowerRoadmapClassification { .. } => {
                WorkManagementH2aIntentKind::LowerRoadmapClassification
            }
            Self::LowerKpiClassification { .. } => {
                WorkManagementH2aIntentKind::LowerKpiClassification
            }
            Self::LowerKpiObservationClassification { .. } => {
                WorkManagementH2aIntentKind::LowerKpiObservationClassification
            }
            Self::LowerActionClassification { .. } => {
                WorkManagementH2aIntentKind::LowerActionClassification
            }
            Self::LowerDecisionClassification { .. } => {
                WorkManagementH2aIntentKind::LowerDecisionClassification
            }
            Self::LowerRiskClassification { .. } => {
                WorkManagementH2aIntentKind::LowerRiskClassification
            }
            Self::LowerIssueClassification { .. } => {
                WorkManagementH2aIntentKind::LowerIssueClassification
            }
            Self::LowerInitiativeClassification { .. } => {
                WorkManagementH2aIntentKind::LowerInitiativeClassification
            }
            Self::LowerProjectClassification { .. } => {
                WorkManagementH2aIntentKind::LowerProjectClassification
            }
            Self::LowerMilestoneClassification { .. } => {
                WorkManagementH2aIntentKind::LowerMilestoneClassification
            }
        }
    }

    const fn support_requirement(&self) -> Option<SupportRequirement> {
        match self {
            Self::CompleteAction { .. }
            | Self::ResolveIssue { .. }
            | Self::CloseIssue { .. }
            | Self::ReopenIssue { .. } => Some(SupportRequirement::EvidenceRequired),
            Self::ResolveDecisionRequest { .. } | Self::SupersedeDecision { .. } => {
                Some(SupportRequirement::EvidenceOrJudgment)
            }
            _ => None,
        }
    }

    const fn required_evidence_role(&self) -> Option<EvidenceRole> {
        match self {
            Self::CompleteAction { .. } => Some(EvidenceRole::ActionCompletion),
            Self::ResolveDecisionRequest { .. } | Self::SupersedeDecision { .. } => {
                Some(EvidenceRole::DecisionResolution)
            }
            Self::ResolveIssue { .. } => Some(EvidenceRole::IssueResolution),
            Self::CloseIssue { .. } => Some(EvidenceRole::IssueClosureVerification),
            Self::ReopenIssue { .. } => Some(EvidenceRole::IssueFailedVerification),
            _ => None,
        }
    }

    fn targets(&self) -> Vec<WorkManagementTarget> {
        match self {
            Self::AcceptActionRequest {
                request_id,
                request_version,
                ..
            } => vec![WorkManagementTarget::ActionRequest(
                request_id.clone(),
                *request_version,
            )],
            Self::ResolveDecisionRequest {
                request_id,
                request_version,
                decision_id,
                resulting_action_requests,
                ..
            } => {
                let mut targets = vec![
                    WorkManagementTarget::DecisionRequest(request_id.clone(), *request_version),
                    WorkManagementTarget::Decision(
                        decision_id.clone(),
                        AggregateVersion::initial(),
                    ),
                ];
                targets.extend(resulting_action_requests.iter().map(|item| {
                    WorkManagementTarget::ActionRequest(
                        item.id.clone(),
                        AggregateVersion::initial(),
                    )
                }));
                targets
            }
            Self::CompleteAction {
                action_id,
                action_version,
            }
            | Self::CancelAction {
                action_id,
                action_version,
                ..
            }
            | Self::ReopenAction {
                action_id,
                action_version,
                ..
            } => vec![WorkManagementTarget::Action(
                action_id.clone(),
                *action_version,
            )],
            Self::SupersedeDecision {
                decision_id,
                decision_version,
                replacement_decision_id,
                replacement_decision_version,
                resulting_action_requests,
                incomplete_downstream,
                ..
            } => {
                let mut targets = vec![
                    WorkManagementTarget::Decision(decision_id.clone(), *decision_version),
                    WorkManagementTarget::Decision(
                        replacement_decision_id.clone(),
                        *replacement_decision_version,
                    ),
                ];
                targets.extend(resulting_action_requests.iter().map(|item| {
                    WorkManagementTarget::ActionRequest(
                        item.id.clone(),
                        AggregateVersion::initial(),
                    )
                }));
                targets.extend(incomplete_downstream.iter().map(|item| match item {
                    IncompleteDownstreamWork::ActionRequest(id, version, _) => {
                        WorkManagementTarget::ActionRequest(id.clone(), *version)
                    }
                    IncompleteDownstreamWork::Action(id, version, _) => {
                        WorkManagementTarget::Action(id.clone(), *version)
                    }
                }));
                targets
            }
            Self::RecordRiskOccurrence {
                risk_id,
                risk_version,
                ..
            }
            | Self::CloseRisk {
                risk_id,
                risk_version,
                ..
            } => vec![WorkManagementTarget::Risk(risk_id.clone(), *risk_version)],
            Self::ResolveIssue {
                issue_id,
                issue_version,
                ..
            }
            | Self::CloseIssue {
                issue_id,
                issue_version,
            }
            | Self::ReopenIssue {
                issue_id,
                issue_version,
                ..
            } => vec![WorkManagementTarget::Issue(
                issue_id.clone(),
                *issue_version,
            )],
            Self::LowerPortfolioClassification {
                portfolio_id,
                portfolio_version,
                ..
            } => vec![WorkManagementTarget::Portfolio(
                portfolio_id.clone(),
                *portfolio_version,
            )],
            Self::LowerProductClassification {
                product_id,
                product_version,
                ..
            } => vec![WorkManagementTarget::Product(
                product_id.clone(),
                *product_version,
            )],
            Self::LowerRoadmapClassification {
                roadmap_id,
                roadmap_version,
                ..
            } => vec![WorkManagementTarget::Roadmap(
                roadmap_id.clone(),
                *roadmap_version,
            )],
            Self::LowerKpiClassification {
                kpi_id,
                kpi_version,
                ..
            } => vec![WorkManagementTarget::Kpi(kpi_id.clone(), *kpi_version)],
            Self::LowerKpiObservationClassification {
                observation_id,
                observation_version,
                ..
            } => vec![WorkManagementTarget::KpiObservation(
                observation_id.clone(),
                *observation_version,
            )],
            Self::LowerActionClassification {
                action_id,
                action_version,
                ..
            } => vec![WorkManagementTarget::Action(
                action_id.clone(),
                *action_version,
            )],
            Self::LowerDecisionClassification {
                decision_id,
                decision_version,
                ..
            } => vec![WorkManagementTarget::Decision(
                decision_id.clone(),
                *decision_version,
            )],
            Self::LowerRiskClassification {
                risk_id,
                risk_version,
                ..
            } => vec![WorkManagementTarget::Risk(risk_id.clone(), *risk_version)],
            Self::LowerIssueClassification {
                issue_id,
                issue_version,
                ..
            } => vec![WorkManagementTarget::Issue(
                issue_id.clone(),
                *issue_version,
            )],
            Self::LowerInitiativeClassification {
                initiative_id,
                initiative_version,
                ..
            } => vec![WorkManagementTarget::Initiative(
                initiative_id.clone(),
                *initiative_version,
            )],
            Self::LowerProjectClassification {
                project_id,
                project_version,
                ..
            } => vec![WorkManagementTarget::Project(
                project_id.clone(),
                *project_version,
            )],
            Self::LowerMilestoneClassification {
                milestone_id,
                milestone_version,
                ..
            } => vec![WorkManagementTarget::Milestone(
                milestone_id.clone(),
                *milestone_version,
            )],
        }
    }

    fn effects(&self) -> Vec<WorkManagementEffect> {
        match self {
            Self::AcceptActionRequest {
                request_id,
                action_id,
                ..
            } => vec![
                WorkManagementEffect::AcceptActionRequest(request_id.clone()),
                WorkManagementEffect::CreateAction(action_id.clone()),
                WorkManagementEffect::LinkActionRequestToAction(
                    request_id.clone(),
                    action_id.clone(),
                ),
            ],
            Self::ResolveDecisionRequest {
                request_id,
                decision_id,
                resulting_action_requests,
                ..
            } => {
                let mut effects = vec![
                    WorkManagementEffect::ResolveDecisionRequest(request_id.clone()),
                    WorkManagementEffect::CreateDecision(decision_id.clone()),
                    WorkManagementEffect::LinkDecisionRequestToDecision(
                        request_id.clone(),
                        decision_id.clone(),
                    ),
                ];
                for item in resulting_action_requests {
                    effects.push(WorkManagementEffect::CreateResultingActionRequest(
                        item.id.clone(),
                    ));
                    effects.push(WorkManagementEffect::LinkDecisionToActionRequest(
                        decision_id.clone(),
                        item.id.clone(),
                    ));
                }
                effects
            }
            Self::CompleteAction { action_id, .. } => {
                vec![WorkManagementEffect::CompleteAction(action_id.clone())]
            }
            Self::CancelAction { action_id, .. } => {
                vec![WorkManagementEffect::CancelAction(action_id.clone())]
            }
            Self::ReopenAction { action_id, .. } => {
                vec![WorkManagementEffect::ReopenAction(action_id.clone())]
            }
            Self::SupersedeDecision {
                decision_id,
                replacement_decision_id,
                resulting_action_requests,
                incomplete_downstream,
                ..
            } => {
                let mut effects = vec![
                    WorkManagementEffect::SupersedeDecision(decision_id.clone()),
                    WorkManagementEffect::CreateDecision(replacement_decision_id.clone()),
                    WorkManagementEffect::LinkReplacementDecision(
                        decision_id.clone(),
                        replacement_decision_id.clone(),
                    ),
                ];
                for item in resulting_action_requests {
                    effects.push(WorkManagementEffect::CreateResultingActionRequest(
                        item.id.clone(),
                    ));
                    effects.push(WorkManagementEffect::LinkDecisionToActionRequest(
                        replacement_decision_id.clone(),
                        item.id.clone(),
                    ));
                }
                effects.extend(incomplete_downstream.iter().map(|item| match item {
                    IncompleteDownstreamWork::ActionRequest(id, _, _) => {
                        WorkManagementEffect::FlagSupersededPremiseActionRequest(id.clone())
                    }
                    IncompleteDownstreamWork::Action(id, _, _) => {
                        WorkManagementEffect::FlagSupersededPremiseAction(id.clone())
                    }
                }));
                effects
            }
            Self::RecordRiskOccurrence {
                risk_id, issue_id, ..
            } => vec![
                WorkManagementEffect::RecordRiskOccurrence(risk_id.clone()),
                WorkManagementEffect::CreateIssue(issue_id.clone()),
                WorkManagementEffect::LinkRiskToIssue(risk_id.clone(), issue_id.clone()),
            ],
            Self::CloseRisk { risk_id, .. } => {
                vec![WorkManagementEffect::CloseRisk(risk_id.clone())]
            }
            Self::ResolveIssue { issue_id, .. } => {
                vec![WorkManagementEffect::ResolveIssue(issue_id.clone())]
            }
            Self::CloseIssue { issue_id, .. } => {
                vec![WorkManagementEffect::CloseIssue(issue_id.clone())]
            }
            Self::ReopenIssue { issue_id, .. } => {
                vec![WorkManagementEffect::ReopenIssue(issue_id.clone())]
            }
            Self::LowerPortfolioClassification { portfolio_id, .. } => {
                vec![WorkManagementEffect::LowerPortfolioClassification(
                    portfolio_id.clone(),
                )]
            }
            Self::LowerProductClassification { product_id, .. } => {
                vec![WorkManagementEffect::LowerProductClassification(
                    product_id.clone(),
                )]
            }
            Self::LowerRoadmapClassification { roadmap_id, .. } => {
                vec![WorkManagementEffect::LowerRoadmapClassification(
                    roadmap_id.clone(),
                )]
            }
            Self::LowerKpiClassification { kpi_id, .. } => {
                vec![WorkManagementEffect::LowerKpiClassification(kpi_id.clone())]
            }
            Self::LowerKpiObservationClassification { observation_id, .. } => {
                vec![WorkManagementEffect::LowerKpiObservationClassification(
                    observation_id.clone(),
                )]
            }
            Self::LowerActionClassification { action_id, .. } => {
                vec![WorkManagementEffect::LowerActionClassification(
                    action_id.clone(),
                )]
            }
            Self::LowerDecisionClassification { decision_id, .. } => {
                vec![WorkManagementEffect::LowerDecisionClassification(
                    decision_id.clone(),
                )]
            }
            Self::LowerRiskClassification { risk_id, .. } => {
                vec![WorkManagementEffect::LowerRiskClassification(
                    risk_id.clone(),
                )]
            }
            Self::LowerIssueClassification { issue_id, .. } => {
                vec![WorkManagementEffect::LowerIssueClassification(
                    issue_id.clone(),
                )]
            }
            Self::LowerInitiativeClassification { initiative_id, .. } => {
                vec![WorkManagementEffect::LowerInitiativeClassification(
                    initiative_id.clone(),
                )]
            }
            Self::LowerProjectClassification { project_id, .. } => {
                vec![WorkManagementEffect::LowerProjectClassification(
                    project_id.clone(),
                )]
            }
            Self::LowerMilestoneClassification { milestone_id, .. } => {
                vec![WorkManagementEffect::LowerMilestoneClassification(
                    milestone_id.clone(),
                )]
            }
        }
    }

    fn classification_sources(&self) -> Vec<WorkManagementClassificationSource> {
        match self {
            Self::AcceptActionRequest {
                action_classification,
                ..
            } => vec![WorkManagementClassificationSource {
                role: WorkManagementClassificationSourceRole::CreatedAction,
                classification: *action_classification,
            }],
            Self::ResolveDecisionRequest {
                decision_classification,
                resulting_action_requests,
                ..
            } => {
                let mut sources = vec![WorkManagementClassificationSource {
                    role: WorkManagementClassificationSourceRole::CreatedDecision,
                    classification: *decision_classification,
                }];
                sources.extend(resulting_action_requests.iter().map(|item| {
                    WorkManagementClassificationSource {
                        role: WorkManagementClassificationSourceRole::ResultingActionRequest(
                            item.id.clone(),
                        ),
                        classification: item.classification,
                    }
                }));
                sources
            }
            Self::SupersedeDecision {
                replacement_decision_classification,
                resulting_action_requests,
                incomplete_downstream,
                ..
            } => {
                let mut sources = vec![WorkManagementClassificationSource {
                    role: WorkManagementClassificationSourceRole::ReplacementDecision,
                    classification: *replacement_decision_classification,
                }];
                sources.extend(resulting_action_requests.iter().map(|item| {
                    WorkManagementClassificationSource {
                        role: WorkManagementClassificationSourceRole::ResultingActionRequest(
                            item.id.clone(),
                        ),
                        classification: item.classification,
                    }
                }));
                sources.extend(incomplete_downstream.iter().map(|item| match item {
                    IncompleteDownstreamWork::ActionRequest(id, _, classification) => {
                        WorkManagementClassificationSource {
                            role: WorkManagementClassificationSourceRole::DownstreamActionRequest(
                                id.clone(),
                            ),
                            classification: *classification,
                        }
                    }
                    IncompleteDownstreamWork::Action(id, _, classification) => {
                        WorkManagementClassificationSource {
                            role: WorkManagementClassificationSourceRole::DownstreamAction(
                                id.clone(),
                            ),
                            classification: *classification,
                        }
                    }
                }));
                sources
            }
            Self::RecordRiskOccurrence {
                issue_classification,
                ..
            } => vec![WorkManagementClassificationSource {
                role: WorkManagementClassificationSourceRole::CreatedIssue,
                classification: *issue_classification,
            }],
            Self::CancelAction {
                evidence_classifications,
                ..
            }
            | Self::ReopenAction {
                evidence_classifications,
                ..
            } => evidence_classifications
                .iter()
                .map(|binding| WorkManagementClassificationSource {
                    role: WorkManagementClassificationSourceRole::Evidence(
                        binding.evidence_id.clone(),
                    ),
                    classification: binding.classification,
                })
                .collect(),
            _ => Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkManagementPolicyDecision {
    Allowed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkManagementCancellationPolicy {
    NotCancellableAfterSubmit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkManagementAuthority {
    HeadOfProducts,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkManagementClassificationSourceRole {
    PrimaryTarget,
    CreatedAction,
    CreatedDecision,
    ReplacementDecision,
    CreatedIssue,
    DownstreamActionRequest(ActionRequestId),
    DownstreamAction(ActionId),
    ResultingActionRequest(ActionRequestId),
    Evidence(EvidenceReferenceId),
    HumanJudgment,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkManagementClassificationSource {
    role: WorkManagementClassificationSourceRole,
    classification: DataClassification,
}

impl WorkManagementClassificationSource {
    #[must_use]
    pub const fn role(&self) -> &WorkManagementClassificationSourceRole {
        &self.role
    }

    #[must_use]
    pub const fn classification(&self) -> DataClassification {
        self.classification
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkManagementPayloadDigest(String);

impl WorkManagementPayloadDigest {
    #[doc(hidden)]
    pub fn from_persisted(value: String) -> Result<Self, PreparedIntentError> {
        if value.len() != 64
            || !value.bytes().all(|byte| {
                byte.is_ascii_digit() || (byte.is_ascii_lowercase() && byte.is_ascii_hexdigit())
            })
        {
            return Err(PreparedIntentError::InvalidTopology);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The audit code every Action Prepared Intent rejection records.
pub const ACTION_PREPARED_REJECTED_AUDIT_CODE: &str = "action.prepared_rejected";
/// The audit code every Decision Prepared Intent rejection records.
pub const DECISION_PREPARED_REJECTED_AUDIT_CODE: &str = "decision.prepared_rejected";
/// v46: the same zero-effect refusal for a pending Risk occurrence/close
/// preview and for a pending Issue resolve/close/reopen preview.
pub const RISK_PREPARED_REJECTED_AUDIT_CODE: &str = "risk.prepared_rejected";
pub const ISSUE_PREPARED_REJECTED_AUDIT_CODE: &str = "issue.prepared_rejected";

/// The Head of Products' explicit refusal of a Prepared Intent (v45).
///
/// DG3's H2a loop ends in approve or reject; until v45 only approval was
/// durable and a reject silently left the preview pending until expiry. A
/// rejection is a *successful* command, not an execution failure: the
/// intent is consumed at `rejected_at`, one zero-effect audit is recorded,
/// no Approval Receipt is minted and no aggregate changes. Expiry limits
/// approval, not refusal: an expired-but-unconsumed preview can still be
/// rejected, and `expired_at_rejection` says so.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RejectedPreparedIntentOutcome {
    prepared_intent_id: PreparedIntentId,
    rejected_at: UtcTimestamp,
    expired_at_rejection: bool,
    audit_event: AuditEvent,
}

impl RejectedPreparedIntentOutcome {
    #[must_use]
    pub const fn new(
        prepared_intent_id: PreparedIntentId,
        rejected_at: UtcTimestamp,
        expired_at_rejection: bool,
        audit_event: AuditEvent,
    ) -> Self {
        Self {
            prepared_intent_id,
            rejected_at,
            expired_at_rejection,
            audit_event,
        }
    }

    #[must_use]
    pub const fn prepared_intent_id(&self) -> &PreparedIntentId {
        &self.prepared_intent_id
    }

    #[must_use]
    pub const fn rejected_at(&self) -> UtcTimestamp {
        self.rejected_at
    }

    /// Whether the preview had already expired when it was rejected.
    #[must_use]
    pub const fn expired_at_rejection(&self) -> bool {
        self.expired_at_rejection
    }

    #[must_use]
    pub const fn audit_event(&self) -> &AuditEvent {
        &self.audit_event
    }
}

/// The one audit shape a Prepared Intent rejection may record: Head of
/// Products, work management, policy allowed, approval rejected, execution
/// not attempted, no effects. Validators and persistence decoders rebuild
/// the expected audit through this same function and compare wholesale.
/// `None` only when `code` is not a legal audit code.
#[must_use]
pub fn prepared_intent_rejection_audit(
    id: crate::identity::AuditEventId,
    occurred_at: UtcTimestamp,
    code: &str,
    target: crate::audit::AuditTarget,
    correlation_id: crate::identity::CorrelationId,
) -> Option<AuditEvent> {
    let code = crate::audit::AuditEventCode::parse(code).ok()?;
    let disposition = crate::audit::AuditDisposition::new(
        crate::audit::AuditPolicyOutcome::Allowed,
        crate::audit::AuditApprovalOutcome::Rejected,
        crate::audit::AuditExecutionOutcome::NotAttempted,
        crate::audit::AuditEffectScope::None,
        Vec::new(),
    )
    .ok()?;
    Some(AuditEvent::new(
        id,
        occurred_at,
        AuditActor::HeadOfProducts,
        crate::audit::AuditAction::new(crate::audit::AuditModule::WorkManagement, code, target),
        correlation_id,
        disposition,
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreparedIntentError {
    InvalidTopology,
    UnclassifiedBinding,
    ExpiryOverflow,
    UnauthorizedActor,
    InvalidSupport,
    MissingConfirmation,
}

impl Display for PreparedIntentError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidTopology => "operation topology is incomplete",
            Self::UnclassifiedBinding => "prepared intent classification must be explicit",
            Self::ExpiryOverflow => "prepared intent expiry cannot be represented",
            Self::UnauthorizedActor => "approval actor is not authorized",
            Self::InvalidSupport => "support witness does not satisfy this operation",
            Self::MissingConfirmation => "explicit approval confirmation is required",
        })
    }
}

impl Error for PreparedIntentError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkManagementPreview {
    id: PreparedIntentId,
    contract_version: u16,
    operation: WorkManagementOperation,
    targets: Vec<WorkManagementTarget>,
    effects: Vec<WorkManagementEffect>,
    classification: DataClassification,
    classification_sources: Vec<WorkManagementClassificationSource>,
    support: Option<SupportWitness>,
    policy: WorkManagementPolicyDecision,
    expires_at: UtcTimestamp,
    cancellation_policy: WorkManagementCancellationPolicy,
    authority: WorkManagementAuthority,
}

impl WorkManagementPreview {
    #[must_use]
    pub const fn id(&self) -> &PreparedIntentId {
        &self.id
    }
    #[must_use]
    pub const fn contract_version(&self) -> u16 {
        self.contract_version
    }
    #[must_use]
    pub const fn operation(&self) -> &WorkManagementOperation {
        &self.operation
    }
    #[must_use]
    pub fn targets(&self) -> &[WorkManagementTarget] {
        &self.targets
    }
    #[must_use]
    pub fn effects(&self) -> &[WorkManagementEffect] {
        &self.effects
    }
    #[must_use]
    pub const fn classification(&self) -> DataClassification {
        self.classification
    }
    #[must_use]
    pub fn classification_sources(&self) -> &[WorkManagementClassificationSource] {
        &self.classification_sources
    }
    #[must_use]
    pub const fn support(&self) -> Option<&SupportWitness> {
        self.support.as_ref()
    }
    #[must_use]
    pub const fn policy(&self) -> WorkManagementPolicyDecision {
        self.policy
    }
    #[must_use]
    pub const fn expires_at(&self) -> UtcTimestamp {
        self.expires_at
    }
    #[must_use]
    pub const fn cancellation_policy(&self) -> WorkManagementCancellationPolicy {
        self.cancellation_policy
    }
    #[must_use]
    pub const fn authority(&self) -> WorkManagementAuthority {
        self.authority
    }
    #[must_use]
    pub fn payload_digest(&self) -> WorkManagementPayloadDigest {
        calculate_preview_digest(self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkManagementPreparedIntent {
    preview: WorkManagementPreview,
    payload_digest: WorkManagementPayloadDigest,
}

impl WorkManagementPreparedIntent {
    pub fn prepare(
        id: PreparedIntentId,
        mut operation: WorkManagementOperation,
        target_classification: DataClassification,
        support: Option<SupportWitness>,
        now: UtcTimestamp,
    ) -> Result<Self, PreparedIntentError> {
        match &mut operation {
            WorkManagementOperation::ResolveDecisionRequest {
                resulting_action_requests,
                ..
            } => {
                resulting_action_requests.sort_by(|left, right| left.id.cmp(&right.id));
                if resulting_action_requests
                    .windows(2)
                    .any(|pair| pair[0].id == pair[1].id)
                {
                    return Err(PreparedIntentError::InvalidTopology);
                }
            }
            WorkManagementOperation::SupersedeDecision {
                replacement_decision_version,
                resulting_action_requests,
                incomplete_downstream,
                ..
            } => {
                if *replacement_decision_version != AggregateVersion::initial() {
                    return Err(PreparedIntentError::InvalidTopology);
                }
                resulting_action_requests.sort_by(|left, right| left.id.cmp(&right.id));
                incomplete_downstream.sort_by(|left, right| {
                    downstream_id(left)
                        .cmp(downstream_id(right))
                        .then_with(|| downstream_kind(left).cmp(&downstream_kind(right)))
                });
                if resulting_action_requests.windows(2).any(|pair| pair[0].id == pair[1].id)
                    || incomplete_downstream.windows(2).any(|pair| same_downstream_identity(&pair[0], &pair[1]))
                    || resulting_action_requests.iter().any(|resulting| incomplete_downstream.iter().any(|downstream| matches!(downstream, IncompleteDownstreamWork::ActionRequest(id, _, _) if id == &resulting.id)))
                {
                    return Err(PreparedIntentError::InvalidTopology);
                }
            }
            _ => {}
        }
        let evidence_classifications = match &mut operation {
            WorkManagementOperation::CancelAction {
                evidence_classifications,
                ..
            }
            | WorkManagementOperation::ReopenAction {
                evidence_classifications,
                ..
            } => Some(evidence_classifications),
            _ => None,
        };
        if let Some(bindings) = evidence_classifications {
            bindings.sort_by(|left, right| left.evidence_id.cmp(&right.evidence_id));
            if bindings
                .windows(2)
                .any(|pair| pair[0].evidence_id == pair[1].evidence_id)
            {
                return Err(PreparedIntentError::InvalidSupport);
            }
        }
        let expires_at = now
            .unix_millis()
            .checked_add(WORK_MANAGEMENT_H2A_TTL_MILLIS)
            .map(UtcTimestamp::from_unix_millis)
            .ok_or(PreparedIntentError::ExpiryOverflow)?;
        if operation.support_requirement() != support.as_ref().map(|item| item.requirement) {
            return Err(PreparedIntentError::InvalidSupport);
        }
        if let (Some(role), Some(witness)) = (operation.required_evidence_role(), support.as_ref())
        {
            if !witness.evidence.is_empty() && witness.evidence.iter().any(|item| item.role != role)
            {
                return Err(PreparedIntentError::InvalidSupport);
            }
            if !matches!(
                operation,
                WorkManagementOperation::ResolveDecisionRequest { .. }
                    | WorkManagementOperation::SupersedeDecision { .. }
            ) && witness.evidence.is_empty()
            {
                return Err(PreparedIntentError::InvalidSupport);
            }
        }
        if matches!(&operation, WorkManagementOperation::SupersedeDecision { decision_id, replacement_decision_id, .. } if decision_id == replacement_decision_id)
        {
            return Err(PreparedIntentError::InvalidTopology);
        }
        let mut classification_sources = vec![WorkManagementClassificationSource {
            role: WorkManagementClassificationSourceRole::PrimaryTarget,
            classification: target_classification,
        }];
        classification_sources.extend(operation.classification_sources());
        if let Some(witness) = support.as_ref() {
            classification_sources.extend(witness.evidence.iter().map(|item| {
                WorkManagementClassificationSource {
                    role: WorkManagementClassificationSourceRole::Evidence(item.id.clone()),
                    classification: item.classification,
                }
            }));
            classification_sources.extend(witness.judgments.iter().map(|item| {
                WorkManagementClassificationSource {
                    role: WorkManagementClassificationSourceRole::HumanJudgment,
                    classification: item.classification,
                }
            }));
        }
        let source_classifications: Vec<_> = classification_sources
            .iter()
            .map(WorkManagementClassificationSource::classification)
            .collect();
        if target_classification == DataClassification::Unclassified
            || source_classifications.contains(&DataClassification::Unclassified)
            || support
                .as_ref()
                .is_some_and(|item| item.classification == DataClassification::Unclassified)
        {
            return Err(PreparedIntentError::UnclassifiedBinding);
        }
        let mut classification = source_classifications
            .into_iter()
            .fold(target_classification, DataClassification::combine);
        if let Some(item) = support.as_ref() {
            classification = classification.combine(item.classification);
        }
        let targets = operation.targets();
        let effects = operation.effects();
        let preview = WorkManagementPreview {
            id,
            contract_version: SUPPORTED_WORK_MANAGEMENT_H2A_CONTRACT_VERSION,
            operation,
            targets,
            effects,
            classification,
            classification_sources,
            support,
            policy: WorkManagementPolicyDecision::Allowed,
            expires_at,
            cancellation_policy: WorkManagementCancellationPolicy::NotCancellableAfterSubmit,
            authority: WorkManagementAuthority::HeadOfProducts,
        };
        let payload_digest = preview.payload_digest();
        Ok(Self {
            preview,
            payload_digest,
        })
    }

    #[must_use]
    pub const fn id(&self) -> &PreparedIntentId {
        self.preview.id()
    }
    #[must_use]
    pub const fn authority(&self) -> WorkManagementAuthority {
        self.preview.authority()
    }
    #[must_use]
    pub const fn cancellation_policy(&self) -> WorkManagementCancellationPolicy {
        self.preview.cancellation_policy()
    }
    #[must_use]
    pub const fn preview(&self) -> &WorkManagementPreview {
        &self.preview
    }
    #[must_use]
    pub const fn payload_digest(&self) -> &WorkManagementPayloadDigest {
        &self.payload_digest
    }
    #[must_use]
    pub const fn classification(&self) -> DataClassification {
        self.preview.classification()
    }
    #[must_use]
    pub const fn operation(&self) -> &WorkManagementOperation {
        self.preview.operation()
    }
    #[must_use]
    pub fn targets(&self) -> Vec<WorkManagementTarget> {
        self.preview.targets.clone()
    }
    #[must_use]
    pub fn effects(&self) -> Vec<WorkManagementEffect> {
        self.preview.effects.clone()
    }
}

fn calculate_preview_digest(preview: &WorkManagementPreview) -> WorkManagementPayloadDigest {
    let mut digest = Sha256::new();
    digest_field(
        &mut digest,
        b"product-mission-control.work-management.prepared-intent.v1",
    );
    digest_field(&mut digest, preview.id.as_str().as_bytes());
    digest_field(&mut digest, &preview.contract_version.to_be_bytes());
    digest_field(
        &mut digest,
        preview.operation.kind().as_persisted().as_bytes(),
    );
    digest_operation(&mut digest, &preview.operation);
    digest_field(&mut digest, &(preview.targets.len() as u64).to_be_bytes());
    for target in &preview.targets {
        digest_target(&mut digest, target);
    }
    digest_field(&mut digest, &(preview.effects.len() as u64).to_be_bytes());
    for effect in &preview.effects {
        digest_effect(&mut digest, effect);
    }
    digest_field(
        &mut digest,
        &(preview.classification_sources.len() as u64).to_be_bytes(),
    );
    for source in &preview.classification_sources {
        digest_classification_source(&mut digest, source);
    }
    digest_field(
        &mut digest,
        preview.classification.as_persisted().as_bytes(),
    );
    digest_support(&mut digest, preview.support.as_ref());
    digest_field(&mut digest, b"allowed");
    digest_field(&mut digest, &preview.expires_at.unix_millis().to_be_bytes());
    digest_field(&mut digest, b"not_cancellable_after_submit");
    digest_field(&mut digest, b"head_of_products");
    WorkManagementPayloadDigest(format!("{:x}", digest.finalize()))
}

fn digest_classification_source(digest: &mut Sha256, source: &WorkManagementClassificationSource) {
    match &source.role {
        WorkManagementClassificationSourceRole::PrimaryTarget => {
            digest_field(digest, b"primary_target");
        }
        WorkManagementClassificationSourceRole::CreatedAction => {
            digest_field(digest, b"created_action");
        }
        WorkManagementClassificationSourceRole::CreatedDecision => {
            digest_field(digest, b"created_decision");
        }
        WorkManagementClassificationSourceRole::ReplacementDecision => {
            digest_field(digest, b"replacement_decision");
        }
        WorkManagementClassificationSourceRole::CreatedIssue => {
            digest_field(digest, b"created_issue");
        }
        WorkManagementClassificationSourceRole::DownstreamActionRequest(id) => {
            digest_field(digest, b"downstream_action_request");
            digest_field(digest, id.as_str().as_bytes());
        }
        WorkManagementClassificationSourceRole::DownstreamAction(id) => {
            digest_field(digest, b"downstream_action");
            digest_field(digest, id.as_str().as_bytes());
        }
        WorkManagementClassificationSourceRole::ResultingActionRequest(id) => {
            digest_field(digest, b"resulting_action_request");
            digest_field(digest, id.as_str().as_bytes());
        }
        WorkManagementClassificationSourceRole::Evidence(id) => {
            digest_field(digest, b"evidence");
            digest_field(digest, id.as_str().as_bytes());
        }
        WorkManagementClassificationSourceRole::HumanJudgment => {
            digest_field(digest, b"human_judgment");
        }
    }
    digest_field(digest, source.classification.as_persisted().as_bytes());
}

fn digest_field(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

fn digest_operation(digest: &mut Sha256, operation: &WorkManagementOperation) {
    fn id_version(d: &mut Sha256, id: &str, version: AggregateVersion) {
        digest_field(d, id.as_bytes());
        digest_field(d, &version.get().to_be_bytes());
    }
    match operation {
        WorkManagementOperation::AcceptActionRequest {
            request_id,
            request_version,
            action_id,
            action_classification,
            action_subject,
            commitment_details,
            intended_owner,
            intended_due_at,
        } => {
            id_version(digest, request_id.as_str(), *request_version);
            digest_field(digest, action_id.as_str().as_bytes());
            digest_field(digest, action_classification.as_persisted().as_bytes());
            digest_field(digest, action_subject.as_str().as_bytes());
            digest_field(digest, commitment_details.as_str().as_bytes());
            digest_field(digest, intended_owner.as_str().as_bytes());
            digest_field(digest, &intended_due_at.unix_millis().to_be_bytes());
        }
        WorkManagementOperation::ResolveDecisionRequest {
            request_id,
            request_version,
            decision_id,
            decision_classification,
            statement,
            rationale,
            impact,
            decision_owner,
            decided_at,
            resulting_action_requests,
        } => {
            id_version(digest, request_id.as_str(), *request_version);
            digest_field(digest, decision_id.as_str().as_bytes());
            digest_field(digest, decision_classification.as_persisted().as_bytes());
            digest_field(digest, statement.as_str().as_bytes());
            digest_field(digest, rationale.as_str().as_bytes());
            digest_field(digest, impact.as_str().as_bytes());
            digest_field(digest, decision_owner.as_str().as_bytes());
            digest_field(digest, &decided_at.unix_millis().to_be_bytes());
            digest_resulting_action_requests(digest, resulting_action_requests);
        }
        WorkManagementOperation::CompleteAction {
            action_id,
            action_version,
        } => id_version(digest, action_id.as_str(), *action_version),
        WorkManagementOperation::CancelAction {
            action_id,
            action_version,
            reason,
            evidence_classifications,
        } => {
            id_version(digest, action_id.as_str(), *action_version);
            digest_field(digest, reason.as_str().as_bytes());
            digest_evidence_classification_bindings(digest, evidence_classifications);
        }
        WorkManagementOperation::ReopenAction {
            action_id,
            action_version,
            mode,
            reason,
            evidence_classifications,
        } => {
            id_version(digest, action_id.as_str(), *action_version);
            digest_field(
                digest,
                match mode {
                    ActionReopenMode::ReopenCompleted => b"reopen_completed",
                    ActionReopenMode::RestartCancelled => b"restart_cancelled",
                },
            );
            digest_field(digest, reason.as_str().as_bytes());
            digest_evidence_classification_bindings(digest, evidence_classifications);
        }
        WorkManagementOperation::SupersedeDecision {
            decision_id,
            decision_version,
            replacement_decision_id,
            replacement_decision_version,
            replacement_decision_classification,
            replacement_statement,
            replacement_rationale,
            replacement_impact,
            replacement_owner,
            replacement_decided_at,
            resulting_action_requests,
            incomplete_downstream,
        } => {
            id_version(digest, decision_id.as_str(), *decision_version);
            id_version(
                digest,
                replacement_decision_id.as_str(),
                *replacement_decision_version,
            );
            digest_field(digest, replacement_statement.as_str().as_bytes());
            digest_field(digest, replacement_rationale.as_str().as_bytes());
            digest_field(digest, replacement_impact.as_str().as_bytes());
            digest_field(digest, replacement_owner.as_str().as_bytes());
            digest_field(digest, &replacement_decided_at.unix_millis().to_be_bytes());
            digest_resulting_action_requests(digest, resulting_action_requests);
            digest_field(
                digest,
                replacement_decision_classification
                    .as_persisted()
                    .as_bytes(),
            );
            digest_field(digest, &(incomplete_downstream.len() as u64).to_be_bytes());
            for item in incomplete_downstream {
                match item {
                    IncompleteDownstreamWork::ActionRequest(id, version, classification) => {
                        digest_field(digest, b"action_request");
                        id_version(digest, id.as_str(), *version);
                        digest_field(digest, classification.as_persisted().as_bytes());
                    }
                    IncompleteDownstreamWork::Action(id, version, classification) => {
                        digest_field(digest, b"action");
                        id_version(digest, id.as_str(), *version);
                        digest_field(digest, classification.as_persisted().as_bytes());
                    }
                }
            }
        }
        WorkManagementOperation::RecordRiskOccurrence {
            risk_id,
            risk_version,
            issue_id,
            issue_classification,
        } => {
            id_version(digest, risk_id.as_str(), *risk_version);
            digest_field(digest, issue_id.as_str().as_bytes());
            digest_field(digest, issue_classification.as_persisted().as_bytes());
        }
        WorkManagementOperation::CloseRisk {
            risk_id,
            risk_version,
            rationale,
        } => {
            id_version(digest, risk_id.as_str(), *risk_version);
            digest_field(digest, rationale.as_str().as_bytes());
        }
        WorkManagementOperation::ResolveIssue {
            issue_id,
            issue_version,
            resolution_type,
            rationale,
        } => {
            id_version(digest, issue_id.as_str(), *issue_version);
            digest_field(digest, resolution_type.as_persisted().as_bytes());
            digest_field(digest, rationale.as_str().as_bytes());
        }
        WorkManagementOperation::CloseIssue {
            issue_id,
            issue_version,
        }
        | WorkManagementOperation::ReopenIssue {
            issue_id,
            issue_version,
            ..
        } => {
            id_version(digest, issue_id.as_str(), *issue_version);
            if let WorkManagementOperation::ReopenIssue { rationale, .. } = operation {
                digest_field(digest, rationale.as_str().as_bytes());
            }
        }
        WorkManagementOperation::LowerPortfolioClassification {
            portfolio_id,
            portfolio_version,
            current_classification,
            proposed_classification,
            rationale,
        } => {
            id_version(digest, portfolio_id.as_str(), *portfolio_version);
            digest_field(digest, current_classification.as_persisted().as_bytes());
            digest_field(digest, proposed_classification.as_persisted().as_bytes());
            digest_field(digest, rationale.as_str().as_bytes());
        }
        WorkManagementOperation::LowerProductClassification {
            product_id,
            product_version,
            current_classification,
            proposed_classification,
            rationale,
        } => {
            id_version(digest, product_id.as_str(), *product_version);
            digest_field(digest, current_classification.as_persisted().as_bytes());
            digest_field(digest, proposed_classification.as_persisted().as_bytes());
            digest_field(digest, rationale.as_str().as_bytes());
        }
        WorkManagementOperation::LowerRoadmapClassification {
            roadmap_id,
            roadmap_version,
            current_classification,
            proposed_classification,
            rationale,
        } => {
            id_version(digest, roadmap_id.as_str(), *roadmap_version);
            digest_field(digest, current_classification.as_persisted().as_bytes());
            digest_field(digest, proposed_classification.as_persisted().as_bytes());
            digest_field(digest, rationale.as_str().as_bytes());
        }
        WorkManagementOperation::LowerKpiClassification {
            kpi_id,
            kpi_version,
            current_classification,
            proposed_classification,
            rationale,
        } => {
            id_version(digest, kpi_id.as_str(), *kpi_version);
            digest_field(digest, current_classification.as_persisted().as_bytes());
            digest_field(digest, proposed_classification.as_persisted().as_bytes());
            digest_field(digest, rationale.as_str().as_bytes());
        }
        WorkManagementOperation::LowerKpiObservationClassification {
            observation_id,
            observation_version,
            current_classification,
            proposed_classification,
            rationale,
        } => {
            id_version(digest, observation_id.as_str(), *observation_version);
            digest_field(digest, current_classification.as_persisted().as_bytes());
            digest_field(digest, proposed_classification.as_persisted().as_bytes());
            digest_field(digest, rationale.as_str().as_bytes());
        }
        WorkManagementOperation::LowerActionClassification {
            action_id,
            action_version,
            current_classification,
            proposed_classification,
            rationale,
        } => {
            id_version(digest, action_id.as_str(), *action_version);
            digest_field(digest, current_classification.as_persisted().as_bytes());
            digest_field(digest, proposed_classification.as_persisted().as_bytes());
            digest_field(digest, rationale.as_str().as_bytes());
        }
        WorkManagementOperation::LowerDecisionClassification {
            decision_id,
            decision_version,
            current_classification,
            proposed_classification,
            rationale,
        } => {
            id_version(digest, decision_id.as_str(), *decision_version);
            digest_field(digest, current_classification.as_persisted().as_bytes());
            digest_field(digest, proposed_classification.as_persisted().as_bytes());
            digest_field(digest, rationale.as_str().as_bytes());
        }
        WorkManagementOperation::LowerRiskClassification {
            risk_id,
            risk_version,
            current_classification,
            proposed_classification,
            rationale,
        } => {
            id_version(digest, risk_id.as_str(), *risk_version);
            digest_field(digest, current_classification.as_persisted().as_bytes());
            digest_field(digest, proposed_classification.as_persisted().as_bytes());
            digest_field(digest, rationale.as_str().as_bytes());
        }
        WorkManagementOperation::LowerIssueClassification {
            issue_id,
            issue_version,
            current_classification,
            proposed_classification,
            rationale,
        } => {
            id_version(digest, issue_id.as_str(), *issue_version);
            digest_field(digest, current_classification.as_persisted().as_bytes());
            digest_field(digest, proposed_classification.as_persisted().as_bytes());
            digest_field(digest, rationale.as_str().as_bytes());
        }
        WorkManagementOperation::LowerInitiativeClassification {
            initiative_id,
            initiative_version,
            current_classification,
            proposed_classification,
            rationale,
        } => {
            id_version(digest, initiative_id.as_str(), *initiative_version);
            digest_field(digest, current_classification.as_persisted().as_bytes());
            digest_field(digest, proposed_classification.as_persisted().as_bytes());
            digest_field(digest, rationale.as_str().as_bytes());
        }
        WorkManagementOperation::LowerProjectClassification {
            project_id,
            project_version,
            current_classification,
            proposed_classification,
            rationale,
        } => {
            id_version(digest, project_id.as_str(), *project_version);
            digest_field(digest, current_classification.as_persisted().as_bytes());
            digest_field(digest, proposed_classification.as_persisted().as_bytes());
            digest_field(digest, rationale.as_str().as_bytes());
        }
        WorkManagementOperation::LowerMilestoneClassification {
            milestone_id,
            milestone_version,
            current_classification,
            proposed_classification,
            rationale,
        } => {
            id_version(digest, milestone_id.as_str(), *milestone_version);
            digest_field(digest, current_classification.as_persisted().as_bytes());
            digest_field(digest, proposed_classification.as_persisted().as_bytes());
            digest_field(digest, rationale.as_str().as_bytes());
        }
    }
}

fn digest_resulting_action_requests(
    digest: &mut Sha256,
    requests: &[DecisionResultingActionRequest],
) {
    digest_field(digest, &(requests.len() as u64).to_be_bytes());
    for request in requests {
        digest_field(digest, request.id.as_str().as_bytes());
        digest_field(digest, request.subject.as_str().as_bytes());
        digest_field(digest, request.details.as_str().as_bytes());
        digest_field(digest, request.intended_owner.as_str().as_bytes());
        digest_field(digest, &request.due_at.unix_millis().to_be_bytes());
        digest_field(digest, request.classification.as_persisted().as_bytes());
    }
}

fn digest_evidence_classification_bindings(
    digest: &mut Sha256,
    bindings: &[EvidenceClassificationBinding],
) {
    digest_field(digest, &(bindings.len() as u64).to_be_bytes());
    for binding in bindings {
        digest_field(digest, binding.evidence_id.as_str().as_bytes());
        digest_field(digest, binding.classification.as_persisted().as_bytes());
    }
}

fn digest_target(digest: &mut Sha256, target: &WorkManagementTarget) {
    let (kind, id, version) = match target {
        WorkManagementTarget::ActionRequest(id, version) => {
            ("action_request", id.as_str(), *version)
        }
        WorkManagementTarget::Action(id, version) => ("action", id.as_str(), *version),
        WorkManagementTarget::DecisionRequest(id, version) => {
            ("decision_request", id.as_str(), *version)
        }
        WorkManagementTarget::Decision(id, version) => ("decision", id.as_str(), *version),
        WorkManagementTarget::Risk(id, version) => ("risk", id.as_str(), *version),
        WorkManagementTarget::Issue(id, version) => ("issue", id.as_str(), *version),
        WorkManagementTarget::Portfolio(id, version) => ("portfolio", id.as_str(), *version),
        WorkManagementTarget::Product(id, version) => ("product", id.as_str(), *version),
        WorkManagementTarget::Roadmap(id, version) => ("roadmap", id.as_str(), *version),
        WorkManagementTarget::Kpi(id, version) => ("kpi", id.as_str(), *version),
        WorkManagementTarget::KpiObservation(id, version) => {
            ("kpi_observation", id.as_str(), *version)
        }
        WorkManagementTarget::Initiative(id, version) => ("initiative", id.as_str(), *version),
        WorkManagementTarget::Project(id, version) => ("project", id.as_str(), *version),
        WorkManagementTarget::Milestone(id, version) => ("milestone", id.as_str(), *version),
    };
    digest_field(digest, kind.as_bytes());
    digest_field(digest, id.as_bytes());
    digest_field(digest, &version.get().to_be_bytes());
}

fn digest_effect(digest: &mut Sha256, effect: &WorkManagementEffect) {
    match effect {
        WorkManagementEffect::LinkActionRequestToAction(request, action) => {
            digest_field(digest, b"link_action_request_to_action");
            digest_field(digest, request.as_str().as_bytes());
            digest_field(digest, action.as_str().as_bytes());
            return;
        }
        WorkManagementEffect::LinkDecisionRequestToDecision(request, decision) => {
            digest_field(digest, b"link_decision_request_to_decision");
            digest_field(digest, request.as_str().as_bytes());
            digest_field(digest, decision.as_str().as_bytes());
            return;
        }
        WorkManagementEffect::LinkDecisionToActionRequest(decision, request) => {
            digest_field(digest, b"link_decision_to_action_request");
            digest_field(digest, decision.as_str().as_bytes());
            digest_field(digest, request.as_str().as_bytes());
            return;
        }
        WorkManagementEffect::LinkReplacementDecision(original, replacement) => {
            digest_field(digest, b"link_replacement_decision");
            digest_field(digest, original.as_str().as_bytes());
            digest_field(digest, replacement.as_str().as_bytes());
            return;
        }
        WorkManagementEffect::LinkRiskToIssue(risk, issue) => {
            digest_field(digest, b"link_risk_to_issue");
            digest_field(digest, risk.as_str().as_bytes());
            digest_field(digest, issue.as_str().as_bytes());
            return;
        }
        _ => {}
    }
    let (kind, id) = match effect {
        WorkManagementEffect::AcceptActionRequest(id) => ("accept_action_request", id.as_str()),
        WorkManagementEffect::CreateAction(id) => ("create_action", id.as_str()),
        WorkManagementEffect::LinkActionRequestToAction(_, _)
        | WorkManagementEffect::LinkDecisionRequestToDecision(_, _)
        | WorkManagementEffect::LinkDecisionToActionRequest(_, _)
        | WorkManagementEffect::LinkReplacementDecision(_, _)
        | WorkManagementEffect::LinkRiskToIssue(_, _) => {
            unreachable!("link effects returned above")
        }
        WorkManagementEffect::ResolveDecisionRequest(id) => {
            ("resolve_decision_request", id.as_str())
        }
        WorkManagementEffect::CreateDecision(id) => ("create_decision", id.as_str()),
        WorkManagementEffect::CreateResultingActionRequest(id) => {
            ("create_resulting_action_request", id.as_str())
        }
        WorkManagementEffect::CompleteAction(id) => ("complete_action", id.as_str()),
        WorkManagementEffect::CancelAction(id) => ("cancel_action", id.as_str()),
        WorkManagementEffect::ReopenAction(id) => ("reopen_action", id.as_str()),
        WorkManagementEffect::SupersedeDecision(id) => ("supersede_decision", id.as_str()),
        WorkManagementEffect::FlagSupersededPremiseActionRequest(id) => {
            ("flag_superseded_premise_action_request", id.as_str())
        }
        WorkManagementEffect::FlagSupersededPremiseAction(id) => {
            ("flag_superseded_premise_action", id.as_str())
        }
        WorkManagementEffect::RecordRiskOccurrence(id) => ("record_risk_occurrence", id.as_str()),
        WorkManagementEffect::CreateIssue(id) => ("create_issue", id.as_str()),
        WorkManagementEffect::CloseRisk(id) => ("close_risk", id.as_str()),
        WorkManagementEffect::ResolveIssue(id) => ("resolve_issue", id.as_str()),
        WorkManagementEffect::CloseIssue(id) => ("close_issue", id.as_str()),
        WorkManagementEffect::ReopenIssue(id) => ("reopen_issue", id.as_str()),
        WorkManagementEffect::LowerPortfolioClassification(id) => {
            ("lower_portfolio_classification", id.as_str())
        }
        WorkManagementEffect::LowerProductClassification(id) => {
            ("lower_product_classification", id.as_str())
        }
        WorkManagementEffect::LowerRoadmapClassification(id) => {
            ("lower_roadmap_classification", id.as_str())
        }
        WorkManagementEffect::LowerKpiClassification(id) => {
            ("lower_kpi_classification", id.as_str())
        }
        WorkManagementEffect::LowerKpiObservationClassification(id) => {
            ("lower_kpi_observation_classification", id.as_str())
        }
        WorkManagementEffect::LowerActionClassification(id) => {
            ("lower_action_classification", id.as_str())
        }
        WorkManagementEffect::LowerDecisionClassification(id) => {
            ("lower_decision_classification", id.as_str())
        }
        WorkManagementEffect::LowerRiskClassification(id) => {
            ("lower_risk_classification", id.as_str())
        }
        WorkManagementEffect::LowerIssueClassification(id) => {
            ("lower_issue_classification", id.as_str())
        }
        WorkManagementEffect::LowerInitiativeClassification(id) => {
            ("lower_initiative_classification", id.as_str())
        }
        WorkManagementEffect::LowerProjectClassification(id) => {
            ("lower_project_classification", id.as_str())
        }
        WorkManagementEffect::LowerMilestoneClassification(id) => {
            ("lower_milestone_classification", id.as_str())
        }
    };
    digest_field(digest, kind.as_bytes());
    digest_field(digest, id.as_bytes());
}

fn digest_support(digest: &mut Sha256, support: Option<&SupportWitness>) {
    let Some(support) = support else {
        digest_field(digest, b"none");
        return;
    };
    digest_field(digest, support_name(support.disposition).as_bytes());
    digest_field(digest, support.classification.as_persisted().as_bytes());
    digest_field(digest, &(support.evidence.len() as u64).to_be_bytes());
    for item in &support.evidence {
        digest_field(digest, item.id.as_str().as_bytes());
        digest_field(digest, &item.source_version.get().to_be_bytes());
        digest_field(digest, item.classification.as_persisted().as_bytes());
        digest_field(digest, evidence_role_name(item.role).as_bytes());
        match &item.verification {
            EvidenceVerification::Verified {
                verified_at,
                integrity_digest,
            } => {
                digest_field(digest, b"verified");
                digest_field(digest, &verified_at.unix_millis().to_be_bytes());
                digest_field(digest, integrity_digest.as_str().as_bytes());
            }
            EvidenceVerification::DegradedLastVerified {
                last_verified_at,
                integrity_digest,
            } => {
                digest_field(digest, b"degraded_last_verified");
                digest_field(digest, &last_verified_at.unix_millis().to_be_bytes());
                digest_field(digest, integrity_digest.as_str().as_bytes());
            }
            EvidenceVerification::Unverified => digest_field(digest, b"unverified"),
            EvidenceVerification::IntegrityMismatch => digest_field(digest, b"integrity_mismatch"),
            EvidenceVerification::ObservedUnpinned {
                observed_at,
                integrity_digest,
            } => {
                digest_field(digest, b"observed_unpinned");
                digest_field(digest, &observed_at.unix_millis().to_be_bytes());
                digest_field(digest, integrity_digest.as_str().as_bytes());
            }
        }
    }
    digest_field(digest, &(support.judgments.len() as u64).to_be_bytes());
    for judgment in &support.judgments {
        digest_field(digest, b"head_of_products");
        digest_field(digest, b"proceed_with_documented_rationale");
        digest_field(digest, judgment.rationale.as_str().as_bytes());
        digest_field(digest, judgment.classification.as_persisted().as_bytes());
    }
}

const fn support_name(value: SupportDisposition) -> &'static str {
    match value {
        SupportDisposition::EvidenceSatisfied => "evidence_satisfied",
        SupportDisposition::JudgmentSatisfied => "judgment_satisfied",
        SupportDisposition::VerificationPending => "verification_pending",
    }
}

const fn evidence_role_name(value: EvidenceRole) -> &'static str {
    match value {
        EvidenceRole::ActionCompletion => "action_completion",
        EvidenceRole::DecisionResolution => "decision_resolution",
        EvidenceRole::IssueResolution => "issue_resolution",
        EvidenceRole::IssueClosureVerification => "issue_closure_verification",
        EvidenceRole::IssueFailedVerification => "issue_failed_verification",
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkManagementApproval {
    prepared_id: PreparedIntentId,
    actor: AuditActor,
    acknowledged_payload_digest: WorkManagementPayloadDigest,
    idempotency_id: IdempotencyId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalConfirmation {
    Confirmed,
}

impl WorkManagementApproval {
    pub fn new(
        prepared_id: PreparedIntentId,
        actor: AuditActor,
        acknowledged_payload_digest: WorkManagementPayloadDigest,
        idempotency_id: IdempotencyId,
        confirmation: Option<ApprovalConfirmation>,
    ) -> Result<Self, PreparedIntentError> {
        if actor != AuditActor::HeadOfProducts {
            return Err(PreparedIntentError::UnauthorizedActor);
        }
        if confirmation != Some(ApprovalConfirmation::Confirmed) {
            return Err(PreparedIntentError::MissingConfirmation);
        }
        Ok(Self {
            prepared_id,
            actor,
            acknowledged_payload_digest,
            idempotency_id,
        })
    }

    #[must_use]
    pub const fn prepared_id(&self) -> &PreparedIntentId {
        &self.prepared_id
    }
    #[must_use]
    pub const fn actor(&self) -> AuditActor {
        self.actor
    }
    #[must_use]
    pub const fn acknowledged_payload_digest(&self) -> &WorkManagementPayloadDigest {
        &self.acknowledged_payload_digest
    }
    #[must_use]
    pub const fn idempotency_id(&self) -> &IdempotencyId {
        &self.idempotency_id
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(dead_code)] // Consumed by the immediately following Action lifecycle ticket.
pub(crate) struct WorkManagementApprovalReceipt {
    pub(crate) id: ApprovalReceiptId,
    pub(crate) prepared_id: PreparedIntentId,
    pub(crate) digest: WorkManagementPayloadDigest,
    pub(crate) expires_at: UtcTimestamp,
    pub(crate) actor: AuditActor,
    pub(crate) idempotency_id: IdempotencyId,
}

pub trait ApprovalAuthorizationPort {
    fn authorize(&self, actor: AuditActor) -> bool;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DenyWorkManagementApproval;
impl ApprovalAuthorizationPort for DenyWorkManagementApproval {
    fn authorize(&self, _: AuditActor) -> bool {
        false
    }
}

#[allow(dead_code)] // Consumed by the immediately following Action lifecycle ticket.
pub(crate) trait WorkManagementExecutionIdSource {
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError>;
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)] // Consumed by the immediately following Action lifecycle ticket.
pub(crate) enum WorkManagementCurrentPolicy {
    Allowed,
    Denied,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(dead_code)] // Consumed by the immediately following Action lifecycle ticket.
pub(crate) struct WorkManagementAuthoritativeSnapshot {
    pub(crate) operation: WorkManagementOperation,
    pub(crate) classification: DataClassification,
    pub(crate) classification_sources: Vec<WorkManagementClassificationSource>,
    pub(crate) support: Option<SupportWitness>,
    pub(crate) policy: WorkManagementCurrentPolicy,
}

#[cfg(test)]
trait AuthoritativeRevalidationPort {
    fn current_snapshot(
        &self,
        operation: &WorkManagementOperation,
    ) -> WorkManagementAuthoritativeSnapshot;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)] // Consumed by the immediately following Action lifecycle ticket.
pub(crate) enum WorkManagementApprovalValidationError {
    PreparedIntentMismatch,
    Unauthorized,
    DigestMismatch,
    Expired,
    PreviewChanged,
}

#[allow(dead_code)] // Consumed by the immediately following Action lifecycle ticket.
pub(crate) fn validate_and_mint_work_management_h2a_receipt(
    prepared: &WorkManagementPreparedIntent,
    approval: &WorkManagementApproval,
    current: &WorkManagementAuthoritativeSnapshot,
    now: UtcTimestamp,
    receipt_id: ApprovalReceiptId,
    authorization: &impl ApprovalAuthorizationPort,
) -> Result<WorkManagementApprovalReceipt, WorkManagementApprovalValidationError> {
    if approval.prepared_id != prepared.preview.id {
        return Err(WorkManagementApprovalValidationError::PreparedIntentMismatch);
    }
    if approval.actor != AuditActor::HeadOfProducts || !authorization.authorize(approval.actor) {
        return Err(WorkManagementApprovalValidationError::Unauthorized);
    }
    if approval.acknowledged_payload_digest != prepared.payload_digest {
        return Err(WorkManagementApprovalValidationError::DigestMismatch);
    }
    if now >= prepared.preview.expires_at {
        return Err(WorkManagementApprovalValidationError::Expired);
    }
    let expected = WorkManagementAuthoritativeSnapshot {
        operation: prepared.preview.operation.clone(),
        classification: prepared.preview.classification,
        classification_sources: prepared.preview.classification_sources.clone(),
        support: prepared.preview.support.clone(),
        policy: WorkManagementCurrentPolicy::Allowed,
    };
    if current != &expected {
        return Err(WorkManagementApprovalValidationError::PreviewChanged);
    }
    Ok(WorkManagementApprovalReceipt {
        id: receipt_id,
        prepared_id: prepared.preview.id.clone(),
        digest: prepared.payload_digest.clone(),
        expires_at: prepared.preview.expires_at,
        actor: approval.actor,
        idempotency_id: approval.idempotency_id.clone(),
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg(test)]
struct ValidatedH2aOutcome {
    prepared_id: PreparedIntentId,
    receipt_id: ApprovalReceiptId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg(test)]
struct StoredExecution {
    prepared_id: PreparedIntentId,
    digest: WorkManagementPayloadDigest,
    outcome: ValidatedH2aOutcome,
}

#[derive(Clone, Debug, Default)]
#[cfg(test)]
struct ExecutionState {
    pending: HashMap<PreparedIntentId, WorkManagementPreparedIntent>,
    outcomes: HashMap<IdempotencyId, StoredExecution>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg(test)]
enum H2aExecutionError {
    NotPrepared,
    Unauthorized,
    DigestMismatch,
    Expired,
    StaleAuthoritativeSnapshot,
    IdempotencyConflict,
    ReceiptUnavailable,
    CommitFailed,
}

#[cfg(test)]
impl From<WorkManagementApprovalValidationError> for H2aExecutionError {
    fn from(value: WorkManagementApprovalValidationError) -> Self {
        match value {
            WorkManagementApprovalValidationError::PreparedIntentMismatch => Self::NotPrepared,
            WorkManagementApprovalValidationError::Unauthorized => Self::Unauthorized,
            WorkManagementApprovalValidationError::DigestMismatch => Self::DigestMismatch,
            WorkManagementApprovalValidationError::Expired => Self::Expired,
            WorkManagementApprovalValidationError::PreviewChanged => {
                Self::StaleAuthoritativeSnapshot
            }
        }
    }
}

#[cfg(test)]
struct H2aExecutionHarness<C, I, R, Z> {
    clock: C,
    receipt_ids: I,
    revalidation: R,
    authorization: Z,
    state: ExecutionState,
    fail_next_commit: bool,
}

#[cfg(test)]
impl<
        C: Clock,
        I: WorkManagementExecutionIdSource,
        R: AuthoritativeRevalidationPort,
        Z: ApprovalAuthorizationPort,
    > H2aExecutionHarness<C, I, R, Z>
{
    fn new(clock: C, receipt_ids: I, revalidation: R, authorization: Z) -> Self {
        Self {
            clock,
            receipt_ids,
            revalidation,
            authorization,
            state: ExecutionState::default(),
            fail_next_commit: false,
        }
    }

    fn stage_prepared(&mut self, prepared: WorkManagementPreparedIntent) {
        self.state.pending.insert(prepared.id().clone(), prepared);
    }

    fn inject_next_commit_failure(&mut self) {
        self.fail_next_commit = true;
    }

    fn execute_complete_action(
        &mut self,
        approval: WorkManagementApproval,
    ) -> Result<ValidatedH2aOutcome, H2aExecutionError> {
        if let Some(stored) = self.state.outcomes.get(&approval.idempotency_id) {
            return if stored.prepared_id == approval.prepared_id
                && stored.digest == approval.acknowledged_payload_digest
            {
                Ok(stored.outcome.clone())
            } else {
                Err(H2aExecutionError::IdempotencyConflict)
            };
        }
        let prepared = self
            .state
            .pending
            .get(&approval.prepared_id)
            .ok_or(H2aExecutionError::NotPrepared)?;
        if !matches!(
            prepared.preview.operation,
            WorkManagementOperation::CompleteAction { .. }
        ) {
            return Err(H2aExecutionError::NotPrepared);
        }
        let current = self
            .revalidation
            .current_snapshot(&prepared.preview.operation);
        let receipt_id = self
            .receipt_ids
            .next_approval_receipt_id()
            .map_err(|_| H2aExecutionError::ReceiptUnavailable)?;
        let receipt = validate_and_mint_work_management_h2a_receipt(
            prepared,
            &approval,
            &current,
            self.clock.now(),
            receipt_id,
            &self.authorization,
        )?;
        let mut staged = self.state.clone();
        staged.pending.remove(&receipt.prepared_id);
        let outcome = ValidatedH2aOutcome {
            prepared_id: receipt.prepared_id.clone(),
            receipt_id: receipt.id.clone(),
        };
        staged.outcomes.insert(
            receipt.idempotency_id,
            StoredExecution {
                prepared_id: receipt.prepared_id,
                digest: receipt.digest,
                outcome: outcome.clone(),
            },
        );
        let _consumed_inside_transaction = (receipt.expires_at, receipt.actor);
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(H2aExecutionError::CommitFailed);
        }
        self.state = staged;
        Ok(outcome)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod execution_tests {
    use super::*;

    #[derive(Clone, Copy)]
    struct FixedClock(i64);
    impl Clock for FixedClock {
        fn now(&self) -> UtcTimestamp {
            UtcTimestamp::from_unix_millis(self.0)
        }
    }

    struct ReceiptIds(u64);
    impl WorkManagementExecutionIdSource for ReceiptIds {
        fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
            self.0 += 1;
            PreparedIntentId::parse(format!("prepared-{}", self.0))
        }

        fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError> {
            self.0 += 1;
            ApprovalReceiptId::parse(format!("receipt-{}", self.0))
        }
    }
    #[derive(Clone)]
    struct Revalidation(WorkManagementAuthoritativeSnapshot);
    impl AuthoritativeRevalidationPort for Revalidation {
        fn current_snapshot(
            &self,
            _: &WorkManagementOperation,
        ) -> WorkManagementAuthoritativeSnapshot {
            self.0.clone()
        }
    }
    #[derive(Clone, Copy)]
    struct Allow;
    impl ApprovalAuthorizationPort for Allow {
        fn authorize(&self, actor: AuditActor) -> bool {
            actor == AuditActor::HeadOfProducts
        }
    }

    fn witness() -> SupportWitness {
        EvidenceOrJudgment::new(
            vec![EvidenceReferenceMetadata::new(
                EvidenceReferenceId::parse("evidence-execution").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Internal,
                EvidenceRole::ActionCompletion,
                EvidenceVerification::Verified {
                    verified_at: UtcTimestamp::from_unix_millis(10),
                    integrity_digest: IntegrityDigest::parse("a".repeat(64)).unwrap(),
                },
            )],
            vec![],
        )
        .unwrap()
        .evaluate_evidence_required()
        .unwrap()
    }
    fn make_prepared(id: &str) -> WorkManagementPreparedIntent {
        WorkManagementPreparedIntent::prepare(
            PreparedIntentId::parse(id).unwrap(),
            WorkManagementOperation::CompleteAction {
                action_id: ActionId::parse("action-execution").unwrap(),
                action_version: AggregateVersion::initial(),
            },
            DataClassification::Public,
            Some(witness()),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap()
    }
    fn snapshot(prepared: &WorkManagementPreparedIntent) -> WorkManagementAuthoritativeSnapshot {
        WorkManagementAuthoritativeSnapshot {
            operation: prepared.preview.operation.clone(),
            classification: prepared.preview.classification,
            classification_sources: prepared.preview.classification_sources.clone(),
            support: prepared.preview.support.clone(),
            policy: WorkManagementCurrentPolicy::Allowed,
        }
    }
    fn approval(
        prepared: &WorkManagementPreparedIntent,
        idem: &str,
        digest: WorkManagementPayloadDigest,
    ) -> WorkManagementApproval {
        WorkManagementApproval::new(
            prepared.id().clone(),
            AuditActor::HeadOfProducts,
            digest,
            IdempotencyId::parse(idem).unwrap(),
            Some(ApprovalConfirmation::Confirmed),
        )
        .unwrap()
    }
    fn harness(
        prepared: &WorkManagementPreparedIntent,
        now: i64,
    ) -> H2aExecutionHarness<FixedClock, ReceiptIds, Revalidation, Allow> {
        H2aExecutionHarness::new(
            FixedClock(now),
            ReceiptIds(0),
            Revalidation(snapshot(prepared)),
            Allow,
        )
    }

    #[test]
    fn authorization_digest_expiry_and_revalidation_fail_without_receipt() {
        let prepared = make_prepared("prepared-execution");
        let exact = approval(&prepared, "idem-execution", prepared.payload_digest.clone());
        let other = make_prepared("prepared-mismatch");
        assert_eq!(
            validate_and_mint_work_management_h2a_receipt(
                &prepared,
                &approval(&other, "idem-mismatch", prepared.payload_digest.clone()),
                &snapshot(&prepared),
                UtcTimestamp::from_unix_millis(99),
                ApprovalReceiptId::parse("receipt-mismatch").unwrap(),
                &Allow,
            ),
            Err(WorkManagementApprovalValidationError::PreparedIntentMismatch)
        );
        let mut denied = H2aExecutionHarness::new(
            FixedClock(99),
            ReceiptIds(0),
            Revalidation(snapshot(&prepared)),
            DenyWorkManagementApproval,
        );
        denied.stage_prepared(prepared.clone());
        assert_eq!(
            denied.execute_complete_action(exact.clone()),
            Err(H2aExecutionError::Unauthorized)
        );

        let changed = make_prepared("prepared-other");
        let mut hash = harness(&prepared, 99);
        hash.stage_prepared(prepared.clone());
        assert_eq!(
            hash.execute_complete_action(approval(&prepared, "idem-hash", changed.payload_digest)),
            Err(H2aExecutionError::DigestMismatch)
        );

        let mut expired = harness(&prepared, 300_100);
        expired.stage_prepared(prepared.clone());
        assert_eq!(
            expired.execute_complete_action(approval(
                &prepared,
                "idem-expired",
                prepared.payload_digest.clone()
            )),
            Err(H2aExecutionError::Expired)
        );

        let mut stale_snapshots = vec![snapshot(&prepared); 5];
        stale_snapshots[0].operation = WorkManagementOperation::CompleteAction {
            action_id: ActionId::parse("action-execution").unwrap(),
            action_version: AggregateVersion::new(2).unwrap(),
        };
        stale_snapshots[1].classification = DataClassification::Restricted;
        stale_snapshots[2].policy = WorkManagementCurrentPolicy::Denied;
        stale_snapshots[3].support = None;
        stale_snapshots[4].classification_sources[0].classification = DataClassification::Internal;
        for (index, stale) in stale_snapshots.into_iter().enumerate() {
            let mut h =
                H2aExecutionHarness::new(FixedClock(99), ReceiptIds(0), Revalidation(stale), Allow);
            h.stage_prepared(prepared.clone());
            assert_eq!(
                h.execute_complete_action(approval(
                    &prepared,
                    &format!("idem-stale-{index}"),
                    prepared.payload_digest.clone()
                )),
                Err(H2aExecutionError::StaleAuthoritativeSnapshot)
            );
        }
    }

    #[test]
    fn receipt_is_atomic_single_use_and_idempotency_is_global() {
        let prepared = make_prepared("prepared-execution");
        let exact = approval(&prepared, "idem-execution", prepared.payload_digest.clone());
        let mut h = harness(&prepared, 99);
        h.stage_prepared(prepared.clone());
        let first = h.execute_complete_action(exact.clone()).unwrap();
        assert_eq!(h.execute_complete_action(exact), Ok(first.clone()));
        assert_eq!(
            h.execute_complete_action(approval(
                &prepared,
                "different-idem",
                prepared.payload_digest.clone()
            )),
            Err(H2aExecutionError::NotPrepared)
        );

        let other = make_prepared("prepared-other");
        assert_eq!(
            h.execute_complete_action(approval(
                &other,
                "idem-execution",
                other.payload_digest.clone()
            )),
            Err(H2aExecutionError::IdempotencyConflict)
        );

        let mut restarted = harness(&prepared, 99);
        assert_eq!(
            restarted.execute_complete_action(approval(
                &prepared,
                "restart-idem",
                prepared.payload_digest.clone()
            )),
            Err(H2aExecutionError::NotPrepared)
        );
    }

    #[test]
    fn injected_commit_failure_rolls_back_receipt_outcome_and_allows_retry() {
        let prepared = make_prepared("prepared-execution");
        let exact = approval(&prepared, "idem-retry", prepared.payload_digest.clone());
        let mut h = harness(&prepared, 99);
        h.stage_prepared(prepared);
        h.inject_next_commit_failure();
        assert_eq!(
            h.execute_complete_action(exact.clone()),
            Err(H2aExecutionError::CommitFailed)
        );
        assert!(h.execute_complete_action(exact).is_ok());
    }
}
