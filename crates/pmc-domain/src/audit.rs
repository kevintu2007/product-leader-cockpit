use crate::identity::{
    ActionId, ActionRequestId, AuditEventId, CorrelationId, DecisionId, DecisionRequestId,
    EvidenceReferenceId, InitiativeId, IssueId, KpiId, KpiObservationId, MilestoneId, PortfolioId,
    ProductId, ProjectId, RelationshipId, RiskId, RoadmapId, StakeholderId,
};
use crate::time::UtcTimestamp;
use crate::value::ValueErrorKind;
use crate::value::{validate_token, DomainValueError};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

const MAX_AUDIT_CODE_LENGTH: usize = 96;

pub trait AuditEventIdSource {
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditActor {
    HeadOfProducts,
    PolicyAuthorizedSystem,
}

impl AuditActor {
    #[must_use]
    pub const fn as_persisted(self) -> &'static str {
        match self {
            Self::HeadOfProducts => "head_of_products",
            Self::PolicyAuthorizedSystem => "policy_authorized_system",
        }
    }

    pub fn from_persisted(value: &str) -> Result<Self, DomainValueError> {
        match value {
            "head_of_products" => Ok(Self::HeadOfProducts),
            "policy_authorized_system" => Ok(Self::PolicyAuthorizedSystem),
            _ => Err(DomainValueError::new(ValueErrorKind::UnknownPersistedValue)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditModule {
    Portfolio,
    WorkManagement,
    Classification,
    Execution,
}

impl AuditModule {
    #[must_use]
    pub const fn as_persisted(self) -> &'static str {
        match self {
            Self::Portfolio => "portfolio",
            Self::WorkManagement => "work_management",
            Self::Classification => "classification",
            Self::Execution => "execution",
        }
    }

    pub fn from_persisted(value: &str) -> Result<Self, DomainValueError> {
        match value {
            "portfolio" => Ok(Self::Portfolio),
            "work_management" => Ok(Self::WorkManagement),
            "classification" => Ok(Self::Classification),
            "execution" => Ok(Self::Execution),
            _ => Err(DomainValueError::new(ValueErrorKind::UnknownPersistedValue)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditPolicyOutcome {
    NotRequired,
    Allowed,
    Denied,
}

impl AuditPolicyOutcome {
    #[must_use]
    pub const fn as_persisted(self) -> &'static str {
        match self {
            Self::NotRequired => "not_required",
            Self::Allowed => "allowed",
            Self::Denied => "denied",
        }
    }

    pub fn from_persisted(value: &str) -> Result<Self, DomainValueError> {
        match value {
            "not_required" => Ok(Self::NotRequired),
            "allowed" => Ok(Self::Allowed),
            "denied" => Ok(Self::Denied),
            _ => Err(DomainValueError::new(ValueErrorKind::UnknownPersistedValue)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditApprovalOutcome {
    NotRequired,
    Approved,
    Rejected,
}

impl AuditApprovalOutcome {
    #[must_use]
    pub const fn as_persisted(self) -> &'static str {
        match self {
            Self::NotRequired => "not_required",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
        }
    }

    pub fn from_persisted(value: &str) -> Result<Self, DomainValueError> {
        match value {
            "not_required" => Ok(Self::NotRequired),
            "approved" => Ok(Self::Approved),
            "rejected" => Ok(Self::Rejected),
            _ => Err(DomainValueError::new(ValueErrorKind::UnknownPersistedValue)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditExecutionOutcome {
    NotAttempted,
    Succeeded,
    Failed,
    Cancelled,
}

impl AuditExecutionOutcome {
    #[must_use]
    pub const fn as_persisted(self) -> &'static str {
        match self {
            Self::NotAttempted => "not_attempted",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_persisted(value: &str) -> Result<Self, DomainValueError> {
        match value {
            "not_attempted" => Ok(Self::NotAttempted),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(DomainValueError::new(ValueErrorKind::UnknownPersistedValue)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditEffectScope {
    None,
    Complete,
    Partial,
}

impl AuditEffectScope {
    #[must_use]
    pub const fn as_persisted(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Complete => "complete",
            Self::Partial => "partial",
        }
    }

    pub fn from_persisted(value: &str) -> Result<Self, DomainValueError> {
        match value {
            "none" => Ok(Self::None),
            "complete" => Ok(Self::Complete),
            "partial" => Ok(Self::Partial),
            _ => Err(DomainValueError::new(ValueErrorKind::UnknownPersistedValue)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditDispositionError {
    DeniedAdvanced,
    DeniedHasEffects,
    RejectedAdvanced,
    NotAttemptedHasEffects,
    EffectScopeMismatch,
}

impl Display for AuditDispositionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::DeniedAdvanced => "policy denial cannot advance approval or execution",
            Self::DeniedHasEffects => "policy denial cannot have authoritative effects",
            Self::RejectedAdvanced => "approval rejection cannot advance execution",
            Self::NotAttemptedHasEffects => "unattempted execution cannot have effects",
            Self::EffectScopeMismatch => "effect scope does not match actual effects",
        })
    }
}

impl Error for AuditDispositionError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditEventCode(String);

impl AuditEventCode {
    pub fn parse(value: impl Into<String>) -> Result<Self, DomainValueError> {
        let value = value.into();
        validate_token(&value, MAX_AUDIT_CODE_LENGTH, &['-', '_', '.'])?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditEffectCode(String);

impl AuditEffectCode {
    pub fn parse(value: impl Into<String>) -> Result<Self, DomainValueError> {
        let value = value.into();
        validate_token(&value, MAX_AUDIT_CODE_LENGTH, &['-', '_', '.'])?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuditTarget {
    Portfolio(PortfolioId),
    Product(ProductId),
    Initiative(InitiativeId),
    Project(ProjectId),
    Roadmap(RoadmapId),
    Milestone(MilestoneId),
    Kpi(KpiId),
    KpiObservation(KpiObservationId),
    Stakeholder(StakeholderId),
    Relationship(RelationshipId),
    ActionRequest(ActionRequestId),
    Action(ActionId),
    DecisionRequest(DecisionRequestId),
    Decision(DecisionId),
    Risk(RiskId),
    Issue(IssueId),
    EvidenceReference(EvidenceReferenceId),
    /// The managed Projection subtree of the configured Product Vault
    /// (managed projections). Unlike every other variant this carries no aggregate
    /// identifier, because a Projection rebuild does not act on a Ledger
    /// aggregate at all -- its target is the managed file set itself, and
    /// ADR 0008 defines exactly one such subtree per workspace, so there is
    /// nothing to discriminate between today. If multiple Vault bindings
    /// ever become representable, this gains their identifier rather than
    /// borrowing an unrelated aggregate's.
    ManagedProjectionSet,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditAction {
    module: AuditModule,
    code: AuditEventCode,
    target: AuditTarget,
}

impl AuditAction {
    #[must_use]
    pub const fn new(module: AuditModule, code: AuditEventCode, target: AuditTarget) -> Self {
        Self {
            module,
            code,
            target,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditDisposition {
    policy_outcome: AuditPolicyOutcome,
    approval_outcome: AuditApprovalOutcome,
    execution_outcome: AuditExecutionOutcome,
    effect_scope: AuditEffectScope,
    actual_effects: Vec<AuditEffectCode>,
}

impl AuditDisposition {
    pub fn new(
        policy_outcome: AuditPolicyOutcome,
        approval_outcome: AuditApprovalOutcome,
        execution_outcome: AuditExecutionOutcome,
        effect_scope: AuditEffectScope,
        actual_effects: Vec<AuditEffectCode>,
    ) -> Result<Self, AuditDispositionError> {
        if matches!(policy_outcome, AuditPolicyOutcome::Denied) {
            if !matches!(approval_outcome, AuditApprovalOutcome::NotRequired)
                || !matches!(execution_outcome, AuditExecutionOutcome::NotAttempted)
            {
                return Err(AuditDispositionError::DeniedAdvanced);
            }
            if !actual_effects.is_empty() {
                return Err(AuditDispositionError::DeniedHasEffects);
            }
        }
        if matches!(approval_outcome, AuditApprovalOutcome::Rejected)
            && !matches!(execution_outcome, AuditExecutionOutcome::NotAttempted)
        {
            return Err(AuditDispositionError::RejectedAdvanced);
        }
        if matches!(execution_outcome, AuditExecutionOutcome::NotAttempted)
            && (!matches!(effect_scope, AuditEffectScope::None) || !actual_effects.is_empty())
        {
            return Err(AuditDispositionError::NotAttemptedHasEffects);
        }
        if matches!(effect_scope, AuditEffectScope::None) != actual_effects.is_empty() {
            return Err(AuditDispositionError::EffectScopeMismatch);
        }
        Ok(Self {
            policy_outcome,
            approval_outcome,
            execution_outcome,
            effect_scope,
            actual_effects,
        })
    }

    #[must_use]
    pub const fn effect_scope(&self) -> AuditEffectScope {
        self.effect_scope
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditEvent {
    id: AuditEventId,
    occurred_at: UtcTimestamp,
    actor: AuditActor,
    module: AuditModule,
    code: AuditEventCode,
    correlation_id: CorrelationId,
    target: AuditTarget,
    policy_outcome: AuditPolicyOutcome,
    approval_outcome: AuditApprovalOutcome,
    execution_outcome: AuditExecutionOutcome,
    effect_scope: AuditEffectScope,
    actual_effects: Vec<AuditEffectCode>,
}

impl AuditEvent {
    #[must_use]
    pub fn new(
        id: AuditEventId,
        occurred_at: UtcTimestamp,
        actor: AuditActor,
        action: AuditAction,
        correlation_id: CorrelationId,
        disposition: AuditDisposition,
    ) -> Self {
        Self {
            id,
            occurred_at,
            actor,
            module: action.module,
            code: action.code,
            correlation_id,
            target: action.target,
            policy_outcome: disposition.policy_outcome,
            approval_outcome: disposition.approval_outcome,
            execution_outcome: disposition.execution_outcome,
            effect_scope: disposition.effect_scope,
            actual_effects: disposition.actual_effects,
        }
    }

    #[must_use]
    pub const fn id(&self) -> &AuditEventId {
        &self.id
    }

    #[must_use]
    pub const fn occurred_at(&self) -> UtcTimestamp {
        self.occurred_at
    }

    #[must_use]
    pub const fn actor(&self) -> AuditActor {
        self.actor
    }

    #[must_use]
    pub const fn module(&self) -> AuditModule {
        self.module
    }

    #[must_use]
    pub const fn code(&self) -> &AuditEventCode {
        &self.code
    }

    #[must_use]
    pub const fn correlation_id(&self) -> &CorrelationId {
        &self.correlation_id
    }

    #[must_use]
    pub const fn target(&self) -> &AuditTarget {
        &self.target
    }

    #[must_use]
    pub const fn policy_outcome(&self) -> AuditPolicyOutcome {
        self.policy_outcome
    }

    #[must_use]
    pub const fn approval_outcome(&self) -> AuditApprovalOutcome {
        self.approval_outcome
    }

    #[must_use]
    pub const fn execution_outcome(&self) -> AuditExecutionOutcome {
        self.execution_outcome
    }

    #[must_use]
    pub const fn effect_scope(&self) -> AuditEffectScope {
        self.effect_scope
    }

    #[must_use]
    pub fn actual_effects(&self) -> &[AuditEffectCode] {
        &self.actual_effects
    }
}
