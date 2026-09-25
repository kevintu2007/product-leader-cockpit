//! The Ledger read surface that route composition needs for the Executive
//! Cockpit, Portfolio, People and the Product inspector.
//!
//! Deliberately separate from [`crate::projection_source`] rather than an
//! extension of it. That snapshot is the input to the managed projections'
//! versioned field allowlist, and widening it would put Stakeholder and
//! Milestone facts inside the set a published projection file is derived
//! from -- a governance change to what leaves the Ledger, made as a side
//! effect of wanting to render a screen. These two concerns read the same
//! database and answer different questions, so they get different ports.
//!
//! Like the projection port, an implementation must read every collection
//! from **one point in Ledger history** rather than composing independently
//! timed reads, and must order each collection by its own stable identifier
//! so repeated reads of unchanged state compare equal.
//!
//! Every record carries its authoritative version and classification, because
//! S5 composition is required to identify the owning module and source
//! revision of every field it surfaces and cannot invent either.

use crate::classification::DataClassification;
use crate::identity::{
    AggregateVersion, EvidenceReferenceId, InitiativeId, KpiId, KpiObservationId, MilestoneId,
    ProductId, ProjectId, RoadmapId, StakeholderId,
};
use crate::relationships::{RelationshipKind, StakeholderKind, StakeholderRelationshipPurpose};
use crate::time::UtcTimestamp;
use crate::work_management::{
    ActionRequestState, DecisionRequestState, EvidenceRole, EvidenceVerification, IssueState,
};

/// One Stakeholder as the Ledger holds them.
///
/// `display_name` is the Stakeholder's own recorded name. It is a person's
/// name, so it carries the record's classification with it and a surface must
/// respect that rather than treating a name as public by default.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StakeholderReadRecord {
    pub id: StakeholderId,
    pub display_name: String,
    pub kind: StakeholderKind,
    pub classification: DataClassification,
    pub version: AggregateVersion,
}

/// What a Stakeholder is responsible for, or depends on.
///
/// `purpose` is kept rather than flattened: being responsible for a Milestone
/// and depending on one are different relationships to the same subject, and
/// collapsing them would misstate accountability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StakeholderRelationshipReadRecord {
    pub stakeholder_id: StakeholderId,
    /// The subject's aggregate type, in the Ledger's own vocabulary.
    pub subject_type: String,
    pub subject_id: String,
    pub purpose: StakeholderRelationshipPurpose,
    pub classification: DataClassification,
    pub version: AggregateVersion,
}

/// One Milestone, which the Product-health inspector shows beneath its
/// Project.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MilestoneReadRecord {
    pub id: MilestoneId,
    pub project_id: ProjectId,
    pub name: String,
    pub due_at: UtcTimestamp,
    pub classification: DataClassification,
    pub version: AggregateVersion,
}

/// One Action Request, with the state that decides whether it is still
/// outstanding.
///
/// The state is reported rather than filtered here on purpose. "Outstanding"
/// is a composition judgement -- the People routes want only `Open`, while a
/// Work Queue legitimately wants more than that -- and a read surface that
/// pre-filtered would force every consumer to accept one caller's definition
/// or read the table again.
///
/// `intended_owner_id` is optional because a request may genuinely name no
/// owner yet; that absence is the fact behind the
/// `ActionRequestMissingIntendedOwner` attention reason, so it must survive
/// the read rather than being dropped.
///
/// The two dates answer different questions and are kept apart:
/// `response_due_at` is when the owner must answer the request, and
/// `intended_action_due_at` is when the resulting work is promised. A
/// Decision's follow-up request carries only the second.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionRequestReadRecord {
    pub id: String,
    pub title: String,
    pub intended_owner_id: Option<StakeholderId>,
    pub state: ActionRequestState,
    pub response_due_at: Option<UtcTimestamp>,
    pub intended_action_due_at: Option<UtcTimestamp>,
    pub classification: DataClassification,
    pub version: AggregateVersion,
}

/// One Decision Request.
///
/// Kept separate from an Action Request throughout, because asking someone to
/// decide something and asking someone to do something are different asks
/// with different lifecycles. A Work Queue that merged them would tell the
/// reader neither.
///
/// There is deliberately no deadline field. `DecisionRequestAttentionInput`
/// has one, but SQLite persists no decision deadline anywhere -- that was
/// checked rather than assumed -- so carrying one here would be a field with
/// no source. The approaching-deadline and overdue attention reasons for
/// Decision Requests therefore cannot be derived until S2 persists it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionRequestReadRecord {
    pub id: String,
    pub subject: String,
    pub intended_owner_id: Option<StakeholderId>,
    pub state: DecisionRequestState,
    pub classification: DataClassification,
    pub version: AggregateVersion,
}

/// One Issue.
///
/// `source_risk_id` and `recurrence_of_id` are carried rather than dropped:
/// an Issue that came from a Risk, and an Issue that is a recurrence of an
/// earlier one, are materially different from a fresh Issue, and the
/// `IssueRecurrence` attention reason exists because of the second.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssueReadRecord {
    pub id: String,
    pub title: String,
    pub state: IssueState,
    pub source_risk_id: Option<String>,
    pub recurrence_of_id: Option<String>,
    pub classification: DataClassification,
    pub version: AggregateVersion,
}

/// One Risk, by title.
///
/// The projection snapshot carries a Risk's state, review date and revision
/// but not its title, because free text is outside the managed projection
/// allowlist. A Work Queue that lists Risks needs the title to name them, so
/// it is read here instead of widening that allowlist.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskReadRecord {
    pub id: String,
    pub title: String,
    pub classification: DataClassification,
    pub version: AggregateVersion,
}

/// One endpoint of a Portfolio-hierarchy relationship.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationshipEndpointReadRecord {
    /// The aggregate type, in the Ledger's own vocabulary.
    pub target_type: String,
    pub target_id: String,
}

/// One edge of the Portfolio hierarchy as the Ledger holds it (for the S02
/// detail half and the O01 inspector).
///
/// The hierarchy is not stored as parent columns. `initiatives` and
/// `projects` have no parent; only `milestones.project_id` is a direct
/// column. Everything else -- Portfolio to Product, Product to Project,
/// Product to Roadmap, Product to KPI, Initiative to Project -- is a row in
/// `relationships`, and this record is that row.
///
/// The two endpoints are ordered by `target_type`, ascending, and carry no
/// notion of which is the parent. That is deliberate. Every one of the six
/// Portfolio kinds pairs two *different* types, so the pair is unambiguous
/// without an ordinal -- and ordinal is not guaranteed by the schema. It is
/// also deliberate because these edges are associations, not containment:
/// a Project may sit under more than one Initiative and relate to more than
/// one Product, and a record that named a "parent" would assert a uniqueness
/// the Ledger does not hold.
///
/// `kind` is never `StakeholderSubject`; those are read separately as
/// [`StakeholderRelationshipReadRecord`] because they carry a purpose.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortfolioRelationshipReadRecord {
    pub id: String,
    pub kind: RelationshipKind,
    pub endpoints: [RelationshipEndpointReadRecord; 2],
    pub classification: DataClassification,
    pub version: AggregateVersion,
}

impl PortfolioRelationshipReadRecord {
    /// The identifier of the endpoint with this type, if one of the two has
    /// it. A caller that knows the kind knows which two types to ask for.
    #[must_use]
    pub fn endpoint_id(&self, target_type: &str) -> Option<&str> {
        self.endpoints
            .iter()
            .find(|endpoint| endpoint.target_type == target_type)
            .map(|endpoint| endpoint.target_id.as_str())
    }
}

/// One Product, by name.
///
/// The projection snapshot carries a Product's identity, classification and
/// revision but not its name, because a name is not a projection field. The
/// inspector is *about* a Product, so it reads the name here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProductReadRecord {
    pub id: ProductId,
    pub name: String,
    pub classification: DataClassification,
    pub version: AggregateVersion,
}

/// One Initiative, by name. The Product-health inspector reaches Initiatives
/// only indirectly, through the Projects they share with a Product.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitiativeReadRecord {
    pub id: InitiativeId,
    pub name: String,
    pub classification: DataClassification,
    pub version: AggregateVersion,
}

/// One Project, by name and window.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectReadRecord {
    pub id: ProjectId,
    pub name: String,
    pub start_at: UtcTimestamp,
    pub end_at: UtcTimestamp,
    pub classification: DataClassification,
    pub version: AggregateVersion,
}

/// One Roadmap, by name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoadmapReadRecord {
    pub id: RoadmapId,
    pub name: String,
    pub classification: DataClassification,
    pub version: AggregateVersion,
}

/// One KPI definition, by name. No observed value is read anywhere in this
/// snapshot; a value without its definition's target and cadence would
/// invite a reader to judge it, and that judgment belongs to the KPI module.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KpiDefinitionReadRecord {
    pub id: KpiId,
    pub name: String,
    pub classification: DataClassification,
    pub version: AggregateVersion,
}

/// That a KPI was observed, and when -- never what was measured.
///
/// The Executive Lens's Outcome Observability needs only whether each linked
/// KPI definition has an observation and when the latest was made (DG3 S01
/// amendment, §8.2). `value` and `source` are deliberately absent: they are
/// free text a reader would be tempted to judge, and the managed projection
/// contract keeps measured values out of every reviewed read surface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KpiObservationReadRecord {
    pub id: KpiObservationId,
    pub kpi_id: KpiId,
    pub observed_at: UtcTimestamp,
    pub classification: DataClassification,
    pub version: AggregateVersion,
}

/// One Evidence link: a piece of Evidence attached to some target.
///
/// `classification_at_link` is named for what it is. The Ledger records the
/// classification that applied *when the link was made*, and Evidence itself
/// documents that as "at link time". It must not be substituted for the
/// Evidence's current classification, which is on
/// [`EvidenceReferenceReadRecord`], nor for the target's, which is on the
/// target's own record. Three classifications, three facts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceLinkReadRecord {
    pub evidence_id: EvidenceReferenceId,
    /// The target's aggregate type, in the Ledger's own vocabulary.
    pub target_type: String,
    pub target_id: String,
    pub classification_at_link: DataClassification,
    pub linked_at: UtcTimestamp,
}

/// One Evidence reference as it stands now.
///
/// The Vault path is deliberately absent. A surface receives a domain-level
/// open intent, never a raw path, and a read surface that carried the path
/// would be the place a path leaks from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceReferenceReadRecord {
    pub id: EvidenceReferenceId,
    /// `None` when the Ledger holds no role. The column became nullable in
    /// schema v18; a read that assumed it never was would fail the whole
    /// snapshot on the first such row, taking every route with it.
    pub role: Option<EvidenceRole>,
    pub verification: EvidenceVerification,
    /// Whether a fingerprint was pinned when the reference was created.
    /// An unpinned reference can never be `Verified` by observation again,
    /// only `ObservedUnpinned`; a surface needs this fact to say why, and
    /// what would change it. Derived from the pinned algorithm column being
    /// present, never inferred from the verification state -- a `Verified`
    /// row written before v43 for an unpinned reference must still read as
    /// unpinned.
    pub pinned: bool,
    pub classification: DataClassification,
    pub version: AggregateVersion,
    pub updated_at: UtcTimestamp,
}

/// Who owns a work record, for every kind of work record that has an owner.
///
/// The People tab of the Product-health inspector shows work **via the
/// accountability of a person**, never as the Product's own. That needs one
/// answer to "who owns this" across five tables that spell it differently:
/// `actions.owner_id` and `decisions.owner_id` are required,
/// `risks.owner_id` is optional, and the two request tables call it
/// `intended_owner_id`. Reading them as one collection means the adapter has
/// one lookup rather than five special cases, and a kind whose owner column
/// is missing from the read surface cannot be quietly treated as unowned.
///
/// Issues are absent on purpose: the `issues` table has no owner column at
/// all, so no Issue can be carried by anyone. A surface must say that rather
/// than show an empty list that reads as "nothing owned".
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkOwnerReadRecord {
    /// The work record's aggregate type, in the Ledger's own vocabulary.
    pub target_type: String,
    pub target_id: String,
    pub owner_id: StakeholderId,
}

/// One consistent read of everything S5 composition needs beyond the
/// projection snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LedgerCompositionSnapshot {
    pub schema_version: u32,
    pub ledger_revision: u64,
    pub ledger_as_of_utc: UtcTimestamp,
    pub stakeholders: Vec<StakeholderReadRecord>,
    pub stakeholder_relationships: Vec<StakeholderRelationshipReadRecord>,
    pub milestones: Vec<MilestoneReadRecord>,
    pub action_requests: Vec<ActionRequestReadRecord>,
    pub decision_requests: Vec<DecisionRequestReadRecord>,
    pub issues: Vec<IssueReadRecord>,
    /// Every Risk's title, for surfaces that must name a Risk.
    pub risks: Vec<RiskReadRecord>,
    /// The Portfolio hierarchy, for the Product-health inspector.
    pub portfolio_relationships: Vec<PortfolioRelationshipReadRecord>,
    pub products: Vec<ProductReadRecord>,
    pub initiatives: Vec<InitiativeReadRecord>,
    pub projects: Vec<ProjectReadRecord>,
    pub roadmaps: Vec<RoadmapReadRecord>,
    pub kpi_definitions: Vec<KpiDefinitionReadRecord>,
    /// Every KPI observation's existence and time, without its value.
    pub kpi_observations: Vec<KpiObservationReadRecord>,
    /// Every Evidence link, so a surface can find what is attached to any
    /// target. Read whole rather than per target: the snapshot is one
    /// consistent instant, and a per-target query would be a second read.
    pub evidence_links: Vec<EvidenceLinkReadRecord>,
    pub evidence_references: Vec<EvidenceReferenceReadRecord>,
    /// Who owns each owned work record, across every kind that has an owner.
    pub work_owners: Vec<WorkOwnerReadRecord>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompositionSnapshotReadError {
    Unavailable,
    ReadFailed,
}

/// Reads the Stakeholder and Milestone facts S5 composition needs.
///
/// `observed_at` becomes `ledger_as_of_utc` unchanged. The caller supplies it
/// rather than the port reading a wall clock, matching this workspace's rule
/// that `pmc-ledger` never originates a timestamp.
pub trait LedgerSnapshotForCompositionPort {
    fn read_composition_snapshot(
        &self,
        observed_at: UtcTimestamp,
    ) -> Result<LedgerCompositionSnapshot, CompositionSnapshotReadError>;
}
