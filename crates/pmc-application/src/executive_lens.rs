//! The Executive Lens (S01), on the axes the product owner accepted.
//!
//! DG3 froze a Product scatter on Decision Pressure, Outcome Confidence and
//! Evidence Health. The domain cannot produce any of the three per Product:
//! work records have no Product path, and KPI targets and values are free
//! text. The DG3 S01 amendment of 2026-09-15 replaced them with three measures
//! the Ledger can back, and this module computes exactly those:
//!
//! | Axis | Measure | Read from |
//! |---|---|---|
//! | X | Milestone Timing Exposure | `project_product` -> Projects -> `milestones.due_at` |
//! | Y | Outcome Observability | `product_kpi` -> KPI definitions with an observation |
//! | Bubble | Verified Evidence Coverage | `evidence_links` to the Product -> current verification |
//!
//! Three rules shape everything below, all from the amendment:
//!
//! - **Unknown is a state, not a coordinate.** A Product with no linked
//!   Milestone, KPI definition or Evidence is `Unknown` on that measure. It is
//!   never given a reassuring value, and it gets no quadrant.
//! - **Products are placed, not ranked.** Points come back in name order,
//!   which is navigation order and nothing more.
//! - **Every value says where it came from.** Each measure carries the
//!   records that contributed to it, and the point carries its classification
//!   folded across all of them, naming the contributor that forced the fold.
//!
//! A Milestone has a date and no completion state, so a passed date is only
//! ever "a checkpoint date has passed", never "overdue" or "late".
//!
//! Nothing here writes, and nothing here reads a clock: `as_of` is the
//! snapshot's.

use std::collections::{BTreeMap, BTreeSet};

use pmc_domain::classification::DataClassification;
use pmc_domain::composition_source::LedgerCompositionSnapshot;
use pmc_domain::relationships::RelationshipKind;
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::EvidenceVerification;

/// The due-soon window: 14 days, compared as UTC instants (amendment §5).
pub const DUE_SOON_WINDOW_MILLIS: i64 = 14 * 24 * 60 * 60 * 1000;

/// Milestone Timing Exposure, in increasing exposure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimingState {
    /// No linked Milestone.
    Unknown,
    /// Every Milestone is more than the window away.
    Later,
    /// None has passed, and the earliest is within the window.
    DueSoon,
    /// At least one Milestone date is before `as_of`.
    DatePassed,
}

impl TimingState {
    /// A stable identifier for surfaces.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Later => "later",
            Self::DueSoon => "dueSoon",
            Self::DatePassed => "datePassed",
        }
    }

    /// Whether this is the high-exposure half of the Lens (amendment §4).
    #[must_use]
    pub const fn is_high(self) -> bool {
        matches!(self, Self::DueSoon | Self::DatePassed)
    }
}

/// The verification states in the amendment's frozen severity order, most
/// severe first (§5). The order is policy, not the enum's declaration order.
const SEVERITY: [&str; 5] = [
    "integrity_mismatch",
    "unverified",
    "observed_unpinned",
    "degraded_last_verified",
    "verified",
];

fn severity_rank(kind: &str) -> usize {
    SEVERITY
        .iter()
        .position(|candidate| *candidate == kind)
        .unwrap_or(SEVERITY.len())
}

/// One record that contributed to a measure.
///
/// `version` is the record's aggregate version where it has one. An Evidence
/// link has no identity or version of its own, so it is named by the Evidence
/// and the target and carries `None`; the snapshot revision stands for it
/// (amendment §6).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Contribution {
    pub kind: &'static str,
    pub id: String,
    pub version: Option<u64>,
    pub classification: DataClassification,
    /// What the record is doing here, e.g. `project_product` or `milestone`.
    pub role: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimingMeasure {
    pub state: TimingState,
    /// The earliest linked Milestone date, whatever the state.
    pub earliest_due_at: Option<UtcTimestamp>,
    pub milestone_count: usize,
    pub contributions: Vec<Contribution>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservabilityMeasure {
    /// Linked KPI definitions with at least one observation made by `as_of`.
    pub observed: usize,
    /// Linked KPI definitions. Zero means Unknown, not 0%.
    pub defined: usize,
    pub latest_observed_at: Option<UtcTimestamp>,
    pub contributions: Vec<Contribution>,
}

impl ObservabilityMeasure {
    #[must_use]
    pub const fn is_known(&self) -> bool {
        self.defined > 0
    }

    /// At least half of the linked definitions observed (amendment §4).
    #[must_use]
    pub const fn is_high(&self) -> bool {
        self.defined > 0 && self.observed * 2 >= self.defined
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoverageMeasure {
    /// Distinct linked Evidence currently `Verified`.
    pub verified: usize,
    /// Distinct linked Evidence. Zero means Unknown ("no linked Evidence").
    pub linked: usize,
    /// How many linked Evidence references are in each verification state,
    /// in severity order, zero counts omitted.
    pub by_state: Vec<(&'static str, usize)>,
    /// The most severe current state among them, by the frozen order.
    pub worst: Option<&'static str>,
    pub contributions: Vec<Contribution>,
}

/// The accepted quadrant names (DG3), placed by the amended axes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Quadrant {
    /// Low exposure, high observability.
    KeepMomentum,
    /// High exposure, high observability.
    MonitorClosely,
    /// Low exposure, low observability.
    ExploreAndValidate,
    /// High exposure, low observability.
    PrioritizeNow,
}

impl Quadrant {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::KeepMomentum => "keepMomentum",
            Self::MonitorClosely => "monitorClosely",
            Self::ExploreAndValidate => "exploreAndValidate",
            Self::PrioritizeNow => "prioritizeNow",
        }
    }
}

/// One Product on the Lens.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LensPoint {
    pub product_id: String,
    pub product_name: String,
    pub product_version: u64,
    pub product_classification: DataClassification,
    pub timing: TimingMeasure,
    pub observability: ObservabilityMeasure,
    pub coverage: CoverageMeasure,
    /// `None` whenever either axis is Unknown.
    pub quadrant: Option<Quadrant>,
    /// The Product's classification folded across every contribution.
    pub effective_classification: DataClassification,
    /// The contribution that raised the fold, or `None` when the Product's
    /// own classification stands.
    pub classification_forced_by: Option<Contribution>,
    /// Linked Projects that are also linked to another Product.
    pub shared_project_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutiveLens {
    pub as_of: UtcTimestamp,
    pub ledger_revision: u64,
    pub due_soon_window_millis: i64,
    /// In Product name order: navigation, not priority.
    pub points: Vec<LensPoint>,
}

fn product_links<'a>(
    composition: &'a LedgerCompositionSnapshot,
    kind: RelationshipKind,
    product_id: &'a str,
    other_type: &'static str,
) -> impl Iterator<Item = (&'a str, Contribution)> + 'a {
    composition
        .portfolio_relationships
        .iter()
        .filter(move |relationship| relationship.kind == kind)
        .filter(move |relationship| relationship.endpoint_id("product") == Some(product_id))
        .filter_map(move |relationship| {
            relationship.endpoint_id(other_type).map(|other| {
                (
                    other,
                    Contribution {
                        kind: "relationship",
                        id: relationship.id.clone(),
                        version: Some(relationship.version.get()),
                        classification: relationship.classification,
                        role: kind.as_persisted(),
                    },
                )
            })
        })
}

fn timing_for(composition: &LedgerCompositionSnapshot, product_id: &str) -> TimingMeasure {
    let as_of = composition.ledger_as_of_utc;
    let mut contributions = Vec::new();
    let mut project_ids = BTreeSet::new();
    // An edge to a Project the snapshot does not carry stands on nothing: its
    // Milestones must not produce a timing fact, and the edge itself affects
    // no displayed value, so it is not folded either (amendment §5).
    let present: BTreeSet<&str> = composition
        .projects
        .iter()
        .map(|project| project.id.as_str())
        .collect();
    for (project_id, relationship) in product_links(
        composition,
        RelationshipKind::ProjectProduct,
        product_id,
        "project",
    ) {
        if present.contains(project_id) {
            contributions.push(relationship);
            project_ids.insert(project_id);
        }
    }
    for project in &composition.projects {
        if project_ids.contains(project.id.as_str()) {
            contributions.push(Contribution {
                kind: "project",
                id: project.id.as_str().to_owned(),
                version: Some(project.version.get()),
                classification: project.classification,
                role: "project",
            });
        }
    }
    // Keyed by id, so a Milestone reached twice counts once.
    let milestones: BTreeMap<&str, _> = composition
        .milestones
        .iter()
        .filter(|milestone| project_ids.contains(milestone.project_id.as_str()))
        .map(|milestone| (milestone.id.as_str(), milestone))
        .collect();
    for milestone in milestones.values() {
        contributions.push(Contribution {
            kind: "milestone",
            id: milestone.id.as_str().to_owned(),
            version: Some(milestone.version.get()),
            classification: milestone.classification,
            role: "milestone",
        });
    }

    let earliest = milestones
        .values()
        .map(|milestone| milestone.due_at)
        .min_by_key(|due| due.unix_millis());
    let state = match earliest {
        None => TimingState::Unknown,
        Some(due) if due.unix_millis() < as_of.unix_millis() => TimingState::DatePassed,
        Some(due) if due.unix_millis() - as_of.unix_millis() <= DUE_SOON_WINDOW_MILLIS => {
            TimingState::DueSoon
        }
        Some(_) => TimingState::Later,
    };
    TimingMeasure {
        state,
        earliest_due_at: earliest,
        milestone_count: milestones.len(),
        contributions,
    }
}

fn observability_for(
    composition: &LedgerCompositionSnapshot,
    product_id: &str,
) -> ObservabilityMeasure {
    let as_of = composition.ledger_as_of_utc.unix_millis();
    let mut contributions = Vec::new();
    let mut kpi_ids = BTreeSet::new();
    // Only KPIs whose definition the snapshot carries are "linked
    // definitions"; an edge to a missing one, and any observation of it,
    // affects no displayed value (amendment §5).
    let present: BTreeSet<&str> = composition
        .kpi_definitions
        .iter()
        .map(|kpi| kpi.id.as_str())
        .collect();
    for (kpi_id, relationship) in product_links(
        composition,
        RelationshipKind::ProductKpi,
        product_id,
        "kpi_definition",
    ) {
        if present.contains(kpi_id) {
            contributions.push(relationship);
            kpi_ids.insert(kpi_id);
        }
    }
    let mut defined = 0;
    for kpi in &composition.kpi_definitions {
        if kpi_ids.contains(kpi.id.as_str()) {
            defined += 1;
            contributions.push(Contribution {
                kind: "kpi_definition",
                id: kpi.id.as_str().to_owned(),
                version: Some(kpi.version.get()),
                classification: kpi.classification,
                role: "kpi_definition",
            });
        }
    }
    let mut observed_kpis = BTreeSet::new();
    let mut latest: Option<UtcTimestamp> = None;
    for observation in &composition.kpi_observations {
        // Only observations made by `as_of` count: a read at a past instant
        // must not see an observation from its future (amendment §5).
        if !kpi_ids.contains(observation.kpi_id.as_str())
            || observation.observed_at.unix_millis() > as_of
        {
            continue;
        }
        observed_kpis.insert(observation.kpi_id.as_str());
        let later = match latest {
            None => true,
            Some(current) => observation.observed_at.unix_millis() > current.unix_millis(),
        };
        if later {
            latest = Some(observation.observed_at);
        }
        contributions.push(Contribution {
            kind: "kpi_observation",
            id: observation.id.as_str().to_owned(),
            version: Some(observation.version.get()),
            classification: observation.classification,
            role: "kpi_observation",
        });
    }
    let observed = observed_kpis.len();
    ObservabilityMeasure {
        observed,
        defined,
        latest_observed_at: latest,
        contributions,
    }
}

fn coverage_for(composition: &LedgerCompositionSnapshot, product_id: &str) -> CoverageMeasure {
    let mut contributions = Vec::new();
    // Distinct Evidence, not link rows (amendment §5).
    let mut evidence_ids = BTreeSet::new();
    // A link to Evidence the snapshot does not carry counts toward nothing.
    let present: BTreeSet<&str> = composition
        .evidence_references
        .iter()
        .map(|reference| reference.id.as_str())
        .collect();
    for link in &composition.evidence_links {
        if link.target_type != "product"
            || link.target_id != product_id
            || !present.contains(link.evidence_id.as_str())
        {
            continue;
        }
        if evidence_ids.insert(link.evidence_id.as_str()) {
            contributions.push(Contribution {
                kind: "evidence_link",
                id: format!("{} -> product {}", link.evidence_id.as_str(), product_id),
                version: None,
                classification: link.classification_at_link,
                role: "evidence_link",
            });
        }
    }
    let mut counts: BTreeMap<usize, (&'static str, usize)> = BTreeMap::new();
    let mut linked = 0;
    let mut verified = 0;
    for reference in &composition.evidence_references {
        if !evidence_ids.contains(reference.id.as_str()) {
            continue;
        }
        linked += 1;
        let kind = reference.verification.kind_as_persisted();
        if matches!(
            reference.verification,
            EvidenceVerification::Verified { .. }
        ) {
            verified += 1;
        }
        counts.entry(severity_rank(kind)).or_insert((kind, 0)).1 += 1;
        contributions.push(Contribution {
            kind: "evidence_reference",
            id: reference.id.as_str().to_owned(),
            version: Some(reference.version.get()),
            classification: reference.classification,
            role: "evidence_reference",
        });
    }
    let by_state: Vec<(&'static str, usize)> = counts.into_values().collect();
    CoverageMeasure {
        verified,
        linked,
        worst: by_state.first().map(|(kind, _)| *kind),
        by_state,
        contributions,
    }
}

fn quadrant_for(timing: &TimingMeasure, observability: &ObservabilityMeasure) -> Option<Quadrant> {
    if timing.state == TimingState::Unknown || !observability.is_known() {
        return None;
    }
    Some(match (timing.state.is_high(), observability.is_high()) {
        (true, true) => Quadrant::MonitorClosely,
        (true, false) => Quadrant::PrioritizeNow,
        (false, false) => Quadrant::ExploreAndValidate,
        (false, true) => Quadrant::KeepMomentum,
    })
}

/// Composes the Lens from one composition snapshot.
///
/// Everything it reads is in that snapshot, so the Lens is true at exactly
/// one Ledger revision.
#[must_use]
pub fn compose_executive_lens(composition: &LedgerCompositionSnapshot) -> ExecutiveLens {
    // Projects linked to more than one Product.
    let mut products_per_project: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for relationship in &composition.portfolio_relationships {
        if relationship.kind != RelationshipKind::ProjectProduct {
            continue;
        }
        if let (Some(project), Some(product)) = (
            relationship.endpoint_id("project"),
            relationship.endpoint_id("product"),
        ) {
            products_per_project
                .entry(project)
                .or_default()
                .insert(product);
        }
    }

    let mut products: Vec<_> = composition.products.iter().collect();
    products.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.id.as_str().cmp(right.id.as_str()))
    });

    let points = products
        .into_iter()
        .map(|product| {
            let id = product.id.as_str();
            let timing = timing_for(composition, id);
            let observability = observability_for(composition, id);
            let coverage = coverage_for(composition, id);
            let quadrant = quadrant_for(&timing, &observability);

            let mut effective = product.classification;
            let mut forced_by = None;
            for contribution in timing
                .contributions
                .iter()
                .chain(&observability.contributions)
                .chain(&coverage.contributions)
            {
                let folded = effective.combine(contribution.classification);
                if folded != effective {
                    effective = folded;
                    forced_by = Some(contribution.clone());
                }
            }

            let shared_project_ids = timing
                .contributions
                .iter()
                .filter(|contribution| contribution.kind == "project")
                .filter(|contribution| {
                    products_per_project
                        .get(contribution.id.as_str())
                        .is_some_and(|products| products.len() > 1)
                })
                .map(|contribution| contribution.id.clone())
                .collect();

            LensPoint {
                product_id: id.to_owned(),
                product_name: product.name.clone(),
                product_version: product.version.get(),
                product_classification: product.classification,
                timing,
                observability,
                coverage,
                quadrant,
                effective_classification: effective,
                classification_forced_by: forced_by,
                shared_project_ids,
            }
        })
        .collect();

    ExecutiveLens {
        as_of: composition.ledger_as_of_utc,
        ledger_revision: composition.ledger_revision,
        due_soon_window_millis: DUE_SOON_WINDOW_MILLIS,
        points,
    }
}
