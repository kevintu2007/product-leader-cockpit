//! UI-neutral route composition contracts.
//!
//! The route composition specification requires that "every composed field identifies its owner port and
//! source revision", and that composition "may rank, filter, and cache those
//! fields but may not reinterpret or persist them as new authority". The
//! central idea here is that the second half is enforced by the first: a
//! composed value cannot exist without saying where it came from.
//!
//! [`Sourced<T>`] has private fields and one constructor that demands a
//! [`FieldProvenance`]. There is no way to produce a composed field whose
//! origin is unknown, so provenance cannot be "added later" and cannot be
//! quietly dropped when a field is copied between layers. That is deliberate:
//! a provenance field that merely *exists* would be satisfied by an empty
//! string, and this project has already been bitten several times by shapes
//! that no code path actually fills.
//!
//! These are composition contracts, **not** DG3 route DTOs. The DG3 UI
//! contract is frozen and owns the observable route shape; the composition
//! specification explicitly defines composition "without freezing DG3 route
//! DTOs". Field order and decomposition here are implementation details.
//!
//! `GetWorkQueue` is deliberately absent. It belongs to the Work Queue (S03);
//! this module covers only the five queries of the Executive Cockpit,
//! Portfolio, People and the Product inspector.

use pmc_domain::attention::Freshness;
use pmc_domain::classification::DataClassification;
use pmc_domain::work_management::{EvidenceRole, EvidenceVerification};

use crate::people_composition::RelationshipPurpose;
use crate::work_queue_composition::WorkItemKind;
use pmc_domain::time::UtcTimestamp;

use crate::attention_ranking::RankedAttentionItem;

/// The module that owns a fact, and therefore the only one that may change
/// it. Composition reads across these; it never becomes one of them.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum OwnerModule {
    /// Product, Initiative, Portfolio relationships.
    Portfolio,
    /// Project, Roadmap, Milestone.
    Delivery,
    /// Action Requests and Actions.
    ActionManagement,
    /// Decision Requests and Decisions.
    Decisions,
    Risks,
    Issues,
    /// Stakeholders, responsibilities, dependencies, requests.
    Stakeholders,
    /// Product Vault Evidence references and verification state.
    Evidence,
    Kpi,
    /// Review readiness and carry-forward results.
    Review,
}

impl OwnerModule {
    /// The stable identifier used when a surface must name the owner.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Portfolio => "portfolio",
            Self::Delivery => "delivery",
            Self::ActionManagement => "action_management",
            Self::Decisions => "decisions",
            Self::Risks => "risks",
            Self::Issues => "issues",
            Self::Stakeholders => "stakeholders",
            Self::Evidence => "evidence",
            Self::Kpi => "kpi",
            Self::Review => "review",
        }
    }
}

/// Where one composed field came from.
///
/// Carries exactly what the composition specification names: owner module, source identity,
/// authoritative revision, and `as_of`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldProvenance {
    owner: OwnerModule,
    source_record_id: String,
    source_field: &'static str,
    source_revision: u64,
    as_of: UtcTimestamp,
}

impl FieldProvenance {
    /// Records the origin of one composed field.
    ///
    /// `source_field` is `&'static str` on purpose: it names a field in the
    /// owning module's own vocabulary, which is known at compile time. A
    /// runtime string here would let a caller invent a source that does not
    /// exist.
    #[must_use]
    pub fn new(
        owner: OwnerModule,
        source_record_id: impl Into<String>,
        source_field: &'static str,
        source_revision: u64,
        as_of: UtcTimestamp,
    ) -> Self {
        Self {
            owner,
            source_record_id: source_record_id.into(),
            source_field,
            source_revision,
            as_of,
        }
    }

    #[must_use]
    pub const fn owner(&self) -> OwnerModule {
        self.owner
    }

    #[must_use]
    pub fn source_record_id(&self) -> &str {
        &self.source_record_id
    }

    #[must_use]
    pub const fn source_field(&self) -> &'static str {
        self.source_field
    }

    #[must_use]
    pub const fn source_revision(&self) -> u64 {
        self.source_revision
    }

    #[must_use]
    pub const fn as_of(&self) -> UtcTimestamp {
        self.as_of
    }
}

/// A composed value that cannot exist without its provenance.
///
/// This is the whole mechanism. Because the fields are private and the only
/// constructor takes a [`FieldProvenance`], a route surface cannot present a
/// value whose origin is unknown, and refactoring a field from one contract
/// into another carries the provenance with it rather than losing it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sourced<T> {
    value: T,
    provenance: FieldProvenance,
}

impl<T> Sourced<T> {
    #[must_use]
    pub const fn new(value: T, provenance: FieldProvenance) -> Self {
        Self { value, provenance }
    }

    #[must_use]
    pub const fn value(&self) -> &T {
        &self.value
    }

    #[must_use]
    pub const fn provenance(&self) -> &FieldProvenance {
        &self.provenance
    }

    /// Rewrites the value while keeping the origin. Used when composition
    /// reshapes a fact for presentation -- formatting a count, narrowing an
    /// enum -- and must not lose where it came from.
    #[must_use]
    pub fn map<U>(self, transform: impl FnOnce(T) -> U) -> Sourced<U> {
        Sourced {
            value: transform(self.value),
            provenance: self.provenance,
        }
    }
}

/// The explicit states every route and overlay must be able to express, from
/// the frozen DG3 State and Feedback Contract.
///
/// Modelled as one closed enum so a route cannot invent a state, and so a
/// surface that handles states exhaustively fails to compile when the
/// contract gains one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteState {
    Loading,
    Empty,
    Success,
    Stale,
    Degraded,
    Error,
    PartialSuccess,
    Cancelling,
    Cancelled,
    ApprovalRequired,
    ClassificationDenied,
    BackupDue,
    EvidenceVerificationPending,
    OutOfSync,
}

/// The kind of record a composed entity identifies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComposedEntityKind {
    Product,
    Initiative,
    Project,
    Milestone,
    Stakeholder,
    Kpi,
    Roadmap,
    Evidence,
    ActionRequest,
    Action,
    DecisionRequest,
    Risk,
    Issue,
}

/// The envelope every composed entity carries, from the frozen DG3 query
/// contract: stable identity and type, authoritative revision and `as_of`,
/// classification, freshness and degraded context, provenance summary,
/// attention flags, and allowed legal next actions.
///
/// It deliberately holds no SQLite row, filesystem path, shell string or
/// private diagnostic, which DG3 forbids surfacing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComposedEntity {
    pub kind: ComposedEntityKind,
    pub id: String,
    pub owner: OwnerModule,
    pub revision: u64,
    pub as_of: UtcTimestamp,
    pub classification: DataClassification,
    pub freshness: Freshness,
    pub degraded: bool,
    /// Ranked by the accepted attention policy. Empty means nothing needs
    /// attention, which is a fact rather than an absence of one.
    pub attention: Vec<RankedAttentionItem>,
    /// Intents the record's **lifecycle** admits, named in the domain's own
    /// command vocabulary.
    ///
    /// Lifecycle admissibility is strictly weaker than "may be executed now",
    /// and the distinction matters enough to be in the field's name rather
    /// than only in prose. `prepare_complete_action` is lifecycle-legal for an
    /// Action that is `InProgress`, and preparation can still deny it for
    /// missing Evidence; classification, policy and approval are further gates
    /// that DG3 requires H2 execution to revalidate. So a surface must not
    /// render everything listed here as available, only as not-excluded-by-
    /// lifecycle.
    ///
    /// This doc comment previously said "intents the domain currently
    /// permits", which claimed more than any state-derived list can support.
    ///
    /// Empty until the owning slice supplies them; an empty list truthfully
    /// says "none known", and composition may never invent one, because
    /// offering an intent the domain would refuse lets a surface imply
    /// authority it does not have.
    pub lifecycle_legal_intents: Vec<&'static str>,
}

/// Where a list route is within its result set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageState {
    pub offset: usize,
    pub limit: usize,
    pub total: usize,
}

impl PageState {
    #[must_use]
    pub const fn has_more(&self) -> bool {
        self.offset.saturating_add(self.limit) < self.total
    }
}

/// Shared shape of every composition query result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompositionResult<T> {
    pub state: RouteState,
    pub as_of: UtcTimestamp,
    /// The Ledger revision the whole composition was read at, so a surface
    /// can tell that two fields were read from the same instant.
    pub ledger_revision: u64,
    pub body: T,
}

// ---------------------------------------------------------------------------
// The five Cockpit, Portfolio, People and Product composition queries.
//
// These name the shape of each route's composition. They are intentionally
// thin: the Cockpit aggregation and the People pair are implemented
// separately, and each fills its body from the owner ports. Defining a wide body now,
// with nothing producing it, is the failure mode this project has already
// paid for more than once.
// ---------------------------------------------------------------------------

/// A pulse count together with the definition that makes it inspectable.
///
/// DG1 requires the pulse to carry "inspectable definitions": a bare number
/// invites the reader to guess what it counts, and a guessed denominator is
/// how a count becomes a fabricated progress claim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CountedFact {
    count: usize,
    definition: &'static str,
    provenance: FieldProvenance,
}

impl CountedFact {
    #[must_use]
    pub const fn new(count: usize, definition: &'static str, provenance: FieldProvenance) -> Self {
        Self {
            count,
            definition,
            provenance,
        }
    }

    #[must_use]
    pub const fn count(&self) -> usize {
        self.count
    }

    /// What exactly was counted, in the reader's terms.
    #[must_use]
    pub const fn definition(&self) -> &'static str {
        self.definition
    }

    #[must_use]
    pub const fn provenance(&self) -> &FieldProvenance {
        &self.provenance
    }
}

/// Milestone, commitment and KPI counts.
///
/// Deliberately has no ratio, percentage or progress field. The absence is
/// the feature: DG1 forbids an unsupported progress percentage, and a type
/// that cannot express one cannot leak one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortfolioPulse {
    pub milestones: CountedFact,
    /// Accepted Actions only. An Action Request is not a commitment.
    pub commitments: CountedFact,
    pub kpis: CountedFact,
}

/// Whether an approved review period exists to compare against.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PeriodReadiness {
    /// No Weekly Review has been approved yet, so there is no prior period.
    /// This is an ordinary early state, not an error: before the first
    /// review, "nothing to compare against" is the truth.
    NoApprovedPeriod,
    Approved {
        period_id: String,
        approved_at: UtcTimestamp,
    },
}

/// The period-over-period change the Cockpit shows second.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PeriodChange {
    /// Stated rather than shown as zero. A zero delta claims "nothing
    /// changed", which is a different and stronger assertion than "there is
    /// nothing to compare against".
    NotComparable { because: &'static str },
    Comparable {
        period_id: String,
        since: UtcTimestamp,
    },
}

/// The leader-now briefing: one-line conclusion, the management intervention
/// it implies, and the basis a reader can inspect.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeaderBriefing {
    pub conclusion: String,
    /// `None` when nothing needs intervention. Absent rather than an
    /// encouraging sentence, because inventing an action would make the
    /// briefing a recommendation engine rather than a report.
    pub intervention: Option<String>,
    /// Every fact the conclusion rests on, so it can be checked rather than
    /// trusted.
    pub basis: Vec<FieldProvenance>,
}

/// S01: the Executive Exception Lens.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GetExecutiveCockpit {
    pub as_of: UtcTimestamp,
}

/// The Cockpit body, in the order the DG1 brief fixes: Portfolio health,
/// then period-over-period change, then pulse and measurable exceptions, then
/// the highest-impact attention items, then the leader briefing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutiveCockpitBody {
    /// Products ordered Portfolio-first, each carrying its own envelope.
    pub products: Vec<ComposedEntity>,
    /// Second in the sequence. States that no comparison is possible rather
    /// than showing a zero delta, which would be a stronger claim.
    pub period_change: PeriodChange,
    /// Counts only. There is no progress percentage to render.
    pub pulse: PortfolioPulse,
    /// The ranked exceptions across the whole portfolio, highest first.
    pub exceptions: Vec<RankedAttentionItem>,
    pub leader_briefing: LeaderBriefing,
}

/// S02 list half.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GetPortfolioOverview {
    pub as_of: UtcTimestamp,
    pub page: PageState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortfolioOverviewBody {
    pub products: Vec<ComposedEntity>,
    pub page: PageState,
}

/// S02 detail half, and the O01 Product-health inspector.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GetProductDetail {
    pub product_id: String,
    pub as_of: UtcTimestamp,
}

/// One record in the Product's structure: a Project, Milestone, Roadmap,
/// KPI, or Initiative.
///
/// `via` is `Some` only for an Initiative, naming the Project through which
/// it relates to the Product. No relationship kind pairs a Product with an
/// Initiative directly, and a Project can sit under several Initiatives, so
/// the surface must present this as an association reached through a Project
/// and never as containment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructureEntry {
    pub entity: ComposedEntity,
    pub label: String,
    pub via: Option<String>,
}

/// One piece of Evidence linked to the Product.
///
/// Three classifications, three facts: the Evidence record's current
/// classification is on `entity`, the classification recorded when the link
/// was made is `classification_at_link`, and the Product's own is on the
/// Product. None may stand in for another.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceEntry {
    pub entity: ComposedEntity,
    pub role: Option<EvidenceRole>,
    pub verification: EvidenceVerification,
    /// Whether a fingerprint is pinned. Shown beside the verification so an
    /// unpinned reference reads as a limitation with a remedy, not a verdict.
    pub pinned: bool,
    pub classification_at_link: DataClassification,
    pub linked_at: UtcTimestamp,
}

/// One work item a person accountable for the Product currently carries.
///
/// Carried by the person, not owned by the Product. The accepted O01
/// amendment permits work on this surface only under that framing, so the
/// item keeps its own kind, identifier, revision and read time, and its
/// attention flags keep their own targets.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CarriedWorkItem {
    pub kind: WorkItemKind,
    pub id: String,
    pub label: String,
    pub state_label: &'static str,
    pub lifecycle_legal_intents: Vec<&'static str>,
    pub classification: DataClassification,
    pub revision: u64,
    pub as_of: UtcTimestamp,
    pub attention: Vec<RankedAttentionItem>,
}

/// One person accountable for, or dependent on, the Product.
///
/// `other_products_accountable_for` is shown so a reader can calibrate: a
/// person who carries five overdue items across six Products is a different
/// signal from one who carries them for this Product alone, and the surface
/// cannot tell which without the count.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountablePerson {
    pub entity: ComposedEntity,
    pub display_name: String,
    pub purpose: RelationshipPurpose,
    pub other_products_accountable_for: usize,
    pub carried: Vec<CarriedWorkItem>,
}

/// Which child forced the inspector's effective classification above the
/// Product's own.
///
/// Folding upward is the aggregation guard: putting a Product together with
/// what it exposes reveals more than any one field, so the whole is presented
/// at the most restrictive classification of anything shown. `Unclassified`
/// is absorbing. Without this record a reader sees a classification and no
/// way to tell why -- and cannot judge whether removing one child would
/// change it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassificationFold {
    pub forced_by_kind: ComposedEntityKind,
    pub forced_by_id: String,
    pub classification: DataClassification,
}

/// The `影響` element of the inspector.
///
/// A human judgment, never a derived value. No source exists from which an
/// impact could be computed for a Product -- `decisions.impact` exists but a
/// Decision cannot be scoped to a Product -- and the accepted amendment
/// places this judgment with the person, which is also where the room for
/// human downgrade lives. The read surface can only say whether one has been
/// recorded; the write path is its own slice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImpactJudgment {
    Unassessed,
}

/// The composed Product detail and inspector, per the DG3 O01 amendment:
/// Structure, Evidence and People, with work reached only via
/// accountability, and `影響` as a judgment slot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProductDetailBody {
    /// The Product at its **effective** classification.
    pub product: ComposedEntity,
    /// The Product's recorded name. `ComposedEntity` carries identity, not a
    /// name, and an inspector that is about a Product must be able to say
    /// which one without the reader decoding an identifier.
    pub product_label: String,
    pub classification_forced_by: Option<ClassificationFold>,
    pub structure: Vec<StructureEntry>,
    pub evidence: Vec<EvidenceEntry>,
    pub people: Vec<AccountablePerson>,
    /// `發生`: current conditions, each attributed to the record that raised
    /// it. Never a change over time; no history is read.
    pub health_reasons: Vec<Sourced<String>>,
    /// `影響`.
    pub impact: ImpactJudgment,
}

/// S09 list half.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GetPeopleDirectory {
    pub as_of: UtcTimestamp,
    pub page: PageState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeopleDirectoryBody {
    pub stakeholders: Vec<ComposedEntity>,
    pub page: PageState,
}

/// S09 detail half.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GetStakeholderDetail {
    pub stakeholder_id: String,
    pub as_of: UtcTimestamp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StakeholderDetailBody {
    pub stakeholder: ComposedEntity,
    /// What this person is responsible for, each carrying the owning module.
    pub responsibilities: Vec<Sourced<String>>,
    /// Outstanding requests addressed to them.
    pub outstanding_requests: Vec<ComposedEntity>,
}
