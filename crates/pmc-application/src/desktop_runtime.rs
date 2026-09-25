//! Runtime services the desktop write path needs and the domain deliberately
//! does not provide: a clock, and a source of opaque identifiers.
//!
//! The Ledger never originates a timestamp (every writer takes `at`), so the
//! host reads its clock once per command and hands that instant to the
//! whole flow. Identifiers for Actions, Prepared Intents, receipts and audit
//! events are minted here too: the webview never supplies them (DG3
//! Commands, Queries and DTO Boundary), and the domain only consumes them.
//!
//! The identifiers are opaque and unique -- a launch nonce plus a counter,
//! both rendered as hex -- and they are not secrets: nothing in PMC grants
//! authority by knowing an id. What matters is that two launches never
//! collide and that an id says nothing about the record it names.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_domain::actions::ActionServiceIdSource;
use pmc_domain::audit::AuditEventIdSource;
use pmc_domain::decisions::DecisionServiceIdSource;
use pmc_domain::identity::{
    ActionId, ActionRequestId, ApprovalReceiptId, AuditEventId, DecisionId, DecisionRequestId,
    EvidenceReferenceId, InitiativeId, IssueId, KpiId, KpiObservationId, MilestoneId, PortfolioId,
    PreparedIntentId, ProductId, ProjectId, RelationshipId, RiskId, RoadmapId, StakeholderId,
};
use pmc_domain::issues::IssueServiceIdSource;
use pmc_domain::risks::RiskServiceIdSource;
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::DomainValueError;

/// The host's wall clock, read once per command by the caller.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl SystemClock {
    /// The current instant as the domain's timestamp. A clock before the
    /// Unix epoch is treated as the epoch rather than a negative time.
    #[must_use]
    pub fn read(&self) -> UtcTimestamp {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
            });
        UtcTimestamp::from_unix_millis(millis)
    }
}

impl Clock for SystemClock {
    fn now(&self) -> UtcTimestamp {
        self.read()
    }
}

/// A clock frozen at one instant, so a whole command agrees on when it
/// happened. The facade rehydrates the domain service with this rather
/// than with [`SystemClock`]: preparation time, expiry and audit time then
/// all derive from the one instant the host read.
#[derive(Clone, Copy, Debug)]
pub struct FixedClock(pub UtcTimestamp);

impl Clock for FixedClock {
    fn now(&self) -> UtcTimestamp {
        self.0
    }
}

static NEXT: AtomicU64 = AtomicU64::new(1);

/// Mints opaque identifiers: `<prefix>-<launch nonce>-<sequence>`.
#[derive(Debug)]
pub struct OpaqueIdSource {
    launch_nonce: u64,
}

impl Default for OpaqueIdSource {
    fn default() -> Self {
        Self::new()
    }
}

impl OpaqueIdSource {
    #[must_use]
    pub fn new() -> Self {
        let launch_nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos() as u64);
        Self { launch_nonce }
    }

    /// For tests that need reproducible identifiers.
    #[must_use]
    pub const fn with_nonce(launch_nonce: u64) -> Self {
        Self { launch_nonce }
    }

    /// A resulting Action Request id for a Decision resolution: the domain
    /// declares these on the preview, so no service id-source trait names
    /// them; the host mints them like every other id.
    pub fn next_action_request_id(&mut self) -> Result<ActionRequestId, DomainValueError> {
        ActionRequestId::parse(self.token("request"))
    }

    fn token(&mut self, prefix: &str) -> String {
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        format!("{prefix}-{:x}-{sequence:x}", self.launch_nonce)
    }
}

impl ActionServiceIdSource for OpaqueIdSource {
    fn next_action_id(&mut self) -> Result<ActionId, DomainValueError> {
        ActionId::parse(self.token("action"))
    }
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        PreparedIntentId::parse(self.token("prepared"))
    }
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError> {
        ApprovalReceiptId::parse(self.token("receipt"))
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        AuditEventId::parse(self.token("audit"))
    }
}

impl DecisionServiceIdSource for OpaqueIdSource {
    fn next_decision_id(&mut self) -> Result<DecisionId, DomainValueError> {
        DecisionId::parse(self.token("decision"))
    }
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        PreparedIntentId::parse(self.token("prepared"))
    }
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError> {
        ApprovalReceiptId::parse(self.token("receipt"))
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        AuditEventId::parse(self.token("audit"))
    }
}

impl DecisionServiceIdSource for &mut OpaqueIdSource {
    fn next_decision_id(&mut self) -> Result<DecisionId, DomainValueError> {
        DecisionServiceIdSource::next_decision_id(&mut **self)
    }
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        DecisionServiceIdSource::next_prepared_intent_id(&mut **self)
    }
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError> {
        DecisionServiceIdSource::next_approval_receipt_id(&mut **self)
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        DecisionServiceIdSource::next_audit_event_id(&mut **self)
    }
}

impl RiskServiceIdSource for OpaqueIdSource {
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        PreparedIntentId::parse(self.token("prepared"))
    }
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError> {
        ApprovalReceiptId::parse(self.token("receipt"))
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        AuditEventId::parse(self.token("audit"))
    }
}

impl RiskServiceIdSource for &mut OpaqueIdSource {
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        RiskServiceIdSource::next_prepared_intent_id(&mut **self)
    }
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError> {
        RiskServiceIdSource::next_approval_receipt_id(&mut **self)
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        RiskServiceIdSource::next_audit_event_id(&mut **self)
    }
}

impl IssueServiceIdSource for OpaqueIdSource {
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        PreparedIntentId::parse(self.token("prepared"))
    }
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError> {
        ApprovalReceiptId::parse(self.token("receipt"))
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        AuditEventId::parse(self.token("audit"))
    }
}

impl IssueServiceIdSource for &mut OpaqueIdSource {
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        IssueServiceIdSource::next_prepared_intent_id(&mut **self)
    }
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError> {
        IssueServiceIdSource::next_approval_receipt_id(&mut **self)
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        IssueServiceIdSource::next_audit_event_id(&mut **self)
    }
}

/// The Issue a Risk occurrence will create. The host mints it before the
/// preview is built, because the preview names it and the payload digest
/// binds it; the webview never supplies an identity the host will create.
impl OpaqueIdSource {
    pub fn next_issue_id(&mut self) -> Result<IssueId, DomainValueError> {
        IssueId::parse(self.token("issue"))
    }
}

impl AuditEventIdSource for OpaqueIdSource {
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        AuditEventId::parse(self.token("audit"))
    }
}

/// The record ids a record-entry sheet's create needs (slice 6A). Typed,
/// one per kind, all the same opaque shape; the Ledger's reservation keeps
/// one per `clientRequestId`. No generic string minter is exposed.
impl OpaqueIdSource {
    pub fn next_portfolio_id(&mut self) -> Result<PortfolioId, DomainValueError> {
        PortfolioId::parse(self.token("portfolio"))
    }
    pub fn next_product_id(&mut self) -> Result<ProductId, DomainValueError> {
        ProductId::parse(self.token("product"))
    }
    pub fn next_initiative_id(&mut self) -> Result<InitiativeId, DomainValueError> {
        InitiativeId::parse(self.token("initiative"))
    }
    pub fn next_project_id(&mut self) -> Result<ProjectId, DomainValueError> {
        ProjectId::parse(self.token("project"))
    }
    pub fn next_roadmap_id(&mut self) -> Result<RoadmapId, DomainValueError> {
        RoadmapId::parse(self.token("roadmap"))
    }
    pub fn next_milestone_id(&mut self) -> Result<MilestoneId, DomainValueError> {
        MilestoneId::parse(self.token("milestone"))
    }
    pub fn next_kpi_id(&mut self) -> Result<KpiId, DomainValueError> {
        KpiId::parse(self.token("kpi"))
    }
    pub fn next_kpi_observation_id(&mut self) -> Result<KpiObservationId, DomainValueError> {
        KpiObservationId::parse(self.token("observation"))
    }
    pub fn next_stakeholder_id(&mut self) -> Result<StakeholderId, DomainValueError> {
        StakeholderId::parse(self.token("stakeholder"))
    }
    pub fn next_decision_request_id(&mut self) -> Result<DecisionRequestId, DomainValueError> {
        DecisionRequestId::parse(self.token("decision-request"))
    }
    pub fn next_risk_id(&mut self) -> Result<RiskId, DomainValueError> {
        RiskId::parse(self.token("risk"))
    }
    pub fn next_relationship_id(&mut self) -> Result<RelationshipId, DomainValueError> {
        RelationshipId::parse(self.token("relationship"))
    }
    /// Evidence from a file (schema v48).
    pub fn next_evidence_reference_id(&mut self) -> Result<EvidenceReferenceId, DomainValueError> {
        EvidenceReferenceId::parse(self.token("evidence"))
    }
}

/// The domain service takes its id source by value; the host keeps one
/// source for the whole process, so it lends it.
impl ActionServiceIdSource for &mut OpaqueIdSource {
    fn next_action_id(&mut self) -> Result<ActionId, DomainValueError> {
        (**self).next_action_id()
    }
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        ActionServiceIdSource::next_prepared_intent_id(&mut **self)
    }
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError> {
        ActionServiceIdSource::next_approval_receipt_id(&mut **self)
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        ActionServiceIdSource::next_audit_event_id(&mut **self)
    }
}
