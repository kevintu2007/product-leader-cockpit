//! The Executive Lens on the amended S01 axes (DG3 amendment 2026-09-15).
//!
//! Each test pins one rule of the amendment. Several assert what the Lens
//! must *not* say -- a reassuring value for a Product with no data, a passed
//! date read as late work, an observation from the future -- because those
//! are the ways a chart like this quietly lies.

use pmc_application::executive_lens::{
    compose_executive_lens, LensPoint, Quadrant, TimingState, DUE_SOON_WINDOW_MILLIS,
};
use pmc_domain::classification::DataClassification;
use pmc_domain::composition_source::{
    EvidenceLinkReadRecord, EvidenceReferenceReadRecord, KpiDefinitionReadRecord,
    KpiObservationReadRecord, LedgerCompositionSnapshot, MilestoneReadRecord,
    PortfolioRelationshipReadRecord, ProductReadRecord, ProjectReadRecord,
    RelationshipEndpointReadRecord,
};
use pmc_domain::identity::{
    AggregateVersion, EvidenceReferenceId, KpiId, KpiObservationId, MilestoneId, ProductId,
    ProjectId,
};
use pmc_domain::relationships::RelationshipKind;
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::{EvidenceVerification, IntegrityDigest};

const NOW: i64 = 1_700_000_000_000;
const DAY: i64 = 24 * 60 * 60 * 1000;

fn at(millis: i64) -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(millis)
}

fn version(value: u64) -> AggregateVersion {
    AggregateVersion::new(value).unwrap()
}

fn empty() -> LedgerCompositionSnapshot {
    LedgerCompositionSnapshot {
        schema_version: 46,
        ledger_revision: 11,
        ledger_as_of_utc: at(NOW),
        stakeholders: Vec::new(),
        stakeholder_relationships: Vec::new(),
        milestones: Vec::new(),
        action_requests: Vec::new(),
        decision_requests: Vec::new(),
        risks: Vec::new(),
        issues: Vec::new(),
        portfolio_relationships: Vec::new(),
        products: Vec::new(),
        initiatives: Vec::new(),
        projects: Vec::new(),
        roadmaps: Vec::new(),
        kpi_definitions: Vec::new(),
        kpi_observations: Vec::new(),
        evidence_links: Vec::new(),
        evidence_references: Vec::new(),
        work_owners: Vec::new(),
    }
}

fn product(snapshot: &mut LedgerCompositionSnapshot, id: &str, name: &str) {
    snapshot.products.push(ProductReadRecord {
        id: ProductId::parse(id).unwrap(),
        name: name.to_owned(),
        classification: DataClassification::Internal,
        version: version(1),
    });
}

/// A relationship with its two endpoints ordered by type, as the Ledger reads it.
fn relate(
    snapshot: &mut LedgerCompositionSnapshot,
    id: &str,
    kind: RelationshipKind,
    (a_type, a_id): (&str, &str),
    (b_type, b_id): (&str, &str),
) {
    let mut endpoints = [
        RelationshipEndpointReadRecord {
            target_type: a_type.to_owned(),
            target_id: a_id.to_owned(),
        },
        RelationshipEndpointReadRecord {
            target_type: b_type.to_owned(),
            target_id: b_id.to_owned(),
        },
    ];
    endpoints.sort_by(|left, right| left.target_type.cmp(&right.target_type));
    snapshot
        .portfolio_relationships
        .push(PortfolioRelationshipReadRecord {
            id: id.to_owned(),
            kind,
            endpoints,
            classification: DataClassification::Internal,
            version: version(1),
        });
}

fn project(snapshot: &mut LedgerCompositionSnapshot, id: &str, product_id: &str) {
    snapshot.projects.push(ProjectReadRecord {
        id: ProjectId::parse(id).unwrap(),
        name: format!("Synthetic {id}"),
        start_at: at(0),
        end_at: at(NOW + 365 * DAY),
        classification: DataClassification::Internal,
        version: version(1),
    });
    relate(
        snapshot,
        &format!("rel-{id}-{product_id}"),
        RelationshipKind::ProjectProduct,
        ("project", id),
        ("product", product_id),
    );
}

fn milestone(
    snapshot: &mut LedgerCompositionSnapshot,
    id: &str,
    project_id: &str,
    due_at: i64,
    classification: DataClassification,
) {
    snapshot.milestones.push(MilestoneReadRecord {
        id: MilestoneId::parse(id).unwrap(),
        project_id: ProjectId::parse(project_id).unwrap(),
        name: format!("Synthetic {id}"),
        due_at: at(due_at),
        classification,
        version: version(1),
    });
}

fn kpi(snapshot: &mut LedgerCompositionSnapshot, id: &str, product_id: &str) {
    snapshot.kpi_definitions.push(KpiDefinitionReadRecord {
        id: KpiId::parse(id).unwrap(),
        name: format!("Synthetic {id}"),
        classification: DataClassification::Internal,
        version: version(1),
    });
    relate(
        snapshot,
        &format!("rel-{id}-{product_id}"),
        RelationshipKind::ProductKpi,
        ("kpi_definition", id),
        ("product", product_id),
    );
}

fn observation(snapshot: &mut LedgerCompositionSnapshot, id: &str, kpi_id: &str, observed_at: i64) {
    snapshot.kpi_observations.push(KpiObservationReadRecord {
        id: KpiObservationId::parse(id).unwrap(),
        kpi_id: KpiId::parse(kpi_id).unwrap(),
        observed_at: at(observed_at),
        classification: DataClassification::Internal,
        version: version(1),
    });
}

fn digest() -> IntegrityDigest {
    IntegrityDigest::parse("a".repeat(64)).unwrap()
}

fn evidence(
    snapshot: &mut LedgerCompositionSnapshot,
    id: &str,
    product_id: &str,
    verification: EvidenceVerification,
) {
    snapshot
        .evidence_references
        .push(EvidenceReferenceReadRecord {
            id: EvidenceReferenceId::parse(id).unwrap(),
            role: None,
            verification,
            pinned: true,
            classification: DataClassification::Internal,
            version: version(1),
            updated_at: at(NOW - DAY),
        });
    snapshot.evidence_links.push(EvidenceLinkReadRecord {
        evidence_id: EvidenceReferenceId::parse(id).unwrap(),
        target_type: "product".to_owned(),
        target_id: product_id.to_owned(),
        classification_at_link: DataClassification::Internal,
        linked_at: at(NOW - DAY),
    });
}

fn point<'a>(points: &'a [LensPoint], id: &str) -> &'a LensPoint {
    points
        .iter()
        .find(|point| point.product_id == id)
        .unwrap_or_else(|| panic!("{id} is not on the Lens"))
}

// ---------------------------------------------------------------------------
// Unknown is a state, not a coordinate.
// ---------------------------------------------------------------------------

#[test]
fn a_product_with_nothing_linked_is_unknown_on_every_measure_and_has_no_quadrant() {
    let mut snapshot = empty();
    product(&mut snapshot, "product-bare", "Bare");

    let lens = compose_executive_lens(&snapshot);
    let bare = point(&lens.points, "product-bare");

    assert_eq!(bare.timing.state, TimingState::Unknown);
    assert_eq!(bare.timing.earliest_due_at, None);
    assert!(!bare.observability.is_known());
    assert_eq!(bare.coverage.linked, 0);
    assert_eq!(bare.coverage.worst, None);
    assert_eq!(bare.quadrant, None);
    // Its own classification stands; nothing forced it.
    assert_eq!(bare.effective_classification, DataClassification::Internal);
    assert_eq!(bare.classification_forced_by, None);
}

#[test]
fn a_project_without_milestones_gives_no_timing_fact() {
    // All linked Projects lacking Milestones is Unknown, not Later.
    let mut snapshot = empty();
    product(&mut snapshot, "product-1", "One");
    project(&mut snapshot, "project-empty", "product-1");

    let lens = compose_executive_lens(&snapshot);

    assert_eq!(
        point(&lens.points, "product-1").timing.state,
        TimingState::Unknown
    );
}

#[test]
fn one_unknown_axis_is_enough_to_withhold_a_quadrant() {
    let mut snapshot = empty();
    product(&mut snapshot, "product-1", "One");
    project(&mut snapshot, "project-1", "product-1");
    milestone(
        &mut snapshot,
        "milestone-1",
        "project-1",
        NOW - DAY,
        DataClassification::Internal,
    );

    let lens = compose_executive_lens(&snapshot);
    let one = point(&lens.points, "product-1");

    // The known axis keeps its value; the point just has no quadrant.
    assert_eq!(one.timing.state, TimingState::DatePassed);
    assert!(!one.observability.is_known());
    assert_eq!(one.quadrant, None);
}

// ---------------------------------------------------------------------------
// Milestone Timing Exposure and its frozen boundaries.
// ---------------------------------------------------------------------------

fn timing_with_due(due_at: i64) -> TimingState {
    let mut snapshot = empty();
    product(&mut snapshot, "product-1", "One");
    project(&mut snapshot, "project-1", "product-1");
    milestone(
        &mut snapshot,
        "milestone-1",
        "project-1",
        due_at,
        DataClassification::Internal,
    );
    compose_executive_lens(&snapshot).points[0].timing.state
}

#[test]
fn the_timing_boundaries_are_the_ones_the_amendment_froze() {
    assert_eq!(DUE_SOON_WINDOW_MILLIS, 14 * DAY);
    // A date before `as_of` has passed; `as_of` itself has not.
    assert_eq!(timing_with_due(NOW - 1), TimingState::DatePassed);
    assert_eq!(timing_with_due(NOW), TimingState::DueSoon);
    // Exactly fourteen days away is still due soon; one millisecond more is not.
    assert_eq!(timing_with_due(NOW + 14 * DAY), TimingState::DueSoon);
    assert_eq!(timing_with_due(NOW + 14 * DAY + 1), TimingState::Later);
}

#[test]
fn any_passed_date_makes_the_product_date_passed_whatever_else_is_ahead() {
    let mut snapshot = empty();
    product(&mut snapshot, "product-1", "One");
    project(&mut snapshot, "project-1", "product-1");
    milestone(
        &mut snapshot,
        "milestone-late",
        "project-1",
        NOW + 90 * DAY,
        DataClassification::Internal,
    );
    milestone(
        &mut snapshot,
        "milestone-past",
        "project-1",
        NOW - 3 * DAY,
        DataClassification::Internal,
    );

    let one = &compose_executive_lens(&snapshot).points[0];

    assert_eq!(one.timing.state, TimingState::DatePassed);
    assert_eq!(one.timing.earliest_due_at, Some(at(NOW - 3 * DAY)));
    assert_eq!(one.timing.milestone_count, 2);
}

#[test]
fn a_shared_project_counts_for_each_product_and_is_named_as_shared() {
    let mut snapshot = empty();
    product(&mut snapshot, "product-a", "Alpha");
    product(&mut snapshot, "product-b", "Beta");
    project(&mut snapshot, "project-shared", "product-a");
    relate(
        &mut snapshot,
        "rel-shared-b",
        RelationshipKind::ProjectProduct,
        ("project", "project-shared"),
        ("product", "product-b"),
    );
    milestone(
        &mut snapshot,
        "milestone-1",
        "project-shared",
        NOW + DAY,
        DataClassification::Internal,
    );

    let lens = compose_executive_lens(&snapshot);

    for id in ["product-a", "product-b"] {
        let each = point(&lens.points, id);
        assert_eq!(each.timing.state, TimingState::DueSoon);
        assert_eq!(each.timing.milestone_count, 1);
        assert_eq!(each.shared_project_ids, vec!["project-shared".to_owned()]);
    }
}

#[test]
fn a_milestone_reached_twice_is_counted_once() {
    // Two relationships between the same Project and Product must not
    // double-count the Project's Milestones.
    let mut snapshot = empty();
    product(&mut snapshot, "product-1", "One");
    project(&mut snapshot, "project-1", "product-1");
    relate(
        &mut snapshot,
        "rel-duplicate",
        RelationshipKind::ProjectProduct,
        ("project", "project-1"),
        ("product", "product-1"),
    );
    milestone(
        &mut snapshot,
        "milestone-1",
        "project-1",
        NOW + DAY,
        DataClassification::Internal,
    );

    let one = &compose_executive_lens(&snapshot).points[0];

    assert_eq!(one.timing.milestone_count, 1);
    assert!(one.shared_project_ids.is_empty());
}

// ---------------------------------------------------------------------------
// Outcome Observability.
// ---------------------------------------------------------------------------

#[test]
fn observability_counts_linked_definitions_with_an_observation_made_by_as_of() {
    let mut snapshot = empty();
    product(&mut snapshot, "product-1", "One");
    kpi(&mut snapshot, "kpi-observed", "product-1");
    kpi(&mut snapshot, "kpi-future", "product-1");
    kpi(&mut snapshot, "kpi-never", "product-1");
    observation(&mut snapshot, "obs-1", "kpi-observed", NOW - 2 * DAY);
    observation(&mut snapshot, "obs-2", "kpi-observed", NOW - DAY);
    // After `as_of`: a read at this instant must not see it.
    observation(&mut snapshot, "obs-future", "kpi-future", NOW + DAY);

    let one = &compose_executive_lens(&snapshot).points[0];

    assert_eq!(one.observability.defined, 3);
    assert_eq!(one.observability.observed, 1);
    assert_eq!(one.observability.latest_observed_at, Some(at(NOW - DAY)));
    assert!(!one.observability.is_high());
}

#[test]
fn exactly_half_observed_is_high_observability() {
    let mut snapshot = empty();
    product(&mut snapshot, "product-1", "One");
    kpi(&mut snapshot, "kpi-1", "product-1");
    kpi(&mut snapshot, "kpi-2", "product-1");
    observation(&mut snapshot, "obs-1", "kpi-1", NOW - DAY);

    let one = &compose_executive_lens(&snapshot).points[0];

    assert_eq!(
        (one.observability.observed, one.observability.defined),
        (1, 2)
    );
    assert!(one.observability.is_high());
}

#[test]
fn an_observation_made_exactly_at_as_of_counts() {
    let mut snapshot = empty();
    product(&mut snapshot, "product-1", "One");
    kpi(&mut snapshot, "kpi-1", "product-1");
    observation(&mut snapshot, "obs-now", "kpi-1", NOW);

    let one = &compose_executive_lens(&snapshot).points[0];

    assert_eq!(one.observability.observed, 1);
    assert_eq!(one.observability.latest_observed_at, Some(at(NOW)));
}

// ---------------------------------------------------------------------------
// An edge to a record the snapshot does not carry stands on nothing.
// ---------------------------------------------------------------------------

#[test]
fn a_link_to_a_missing_project_produces_no_timing_fact() {
    let mut snapshot = empty();
    product(&mut snapshot, "product-1", "One");
    // The edge and a Milestone exist, but the Project itself is not read.
    relate(
        &mut snapshot,
        "rel-ghost",
        RelationshipKind::ProjectProduct,
        ("project", "project-ghost"),
        ("product", "product-1"),
    );
    milestone(
        &mut snapshot,
        "milestone-ghost",
        "project-ghost",
        NOW - DAY,
        DataClassification::Restricted,
    );

    let one = &compose_executive_lens(&snapshot).points[0];

    assert_eq!(one.timing.state, TimingState::Unknown);
    assert_eq!(one.timing.milestone_count, 0);
    assert!(one.timing.contributions.is_empty());
    assert_eq!(one.effective_classification, DataClassification::Internal);
    assert_eq!(one.classification_forced_by, None);
}

#[test]
fn a_link_to_a_missing_kpi_definition_says_nothing_about_observability() {
    let mut snapshot = empty();
    product(&mut snapshot, "product-1", "One");
    relate(
        &mut snapshot,
        "rel-ghost",
        RelationshipKind::ProductKpi,
        ("kpi_definition", "kpi-ghost"),
        ("product", "product-1"),
    );
    observation(&mut snapshot, "obs-ghost", "kpi-ghost", NOW - DAY);

    let one = &compose_executive_lens(&snapshot).points[0];

    assert!(!one.observability.is_known());
    assert_eq!(one.observability.observed, 0);
    assert_eq!(one.observability.latest_observed_at, None);
    assert!(one.observability.contributions.is_empty());
}

#[test]
fn a_link_to_missing_evidence_counts_toward_nothing() {
    let mut snapshot = empty();
    product(&mut snapshot, "product-1", "One");
    snapshot.evidence_links.push(EvidenceLinkReadRecord {
        evidence_id: EvidenceReferenceId::parse("evidence-ghost").unwrap(),
        target_type: "product".to_owned(),
        target_id: "product-1".to_owned(),
        classification_at_link: DataClassification::Restricted,
        linked_at: at(NOW - DAY),
    });

    let one = &compose_executive_lens(&snapshot).points[0];

    assert_eq!(one.coverage.linked, 0);
    assert!(one.coverage.contributions.is_empty());
    assert_eq!(one.effective_classification, DataClassification::Internal);
}

#[test]
fn an_observation_of_an_unlinked_kpi_does_not_count() {
    let mut snapshot = empty();
    product(&mut snapshot, "product-1", "One");
    product(&mut snapshot, "product-2", "Two");
    kpi(&mut snapshot, "kpi-1", "product-1");
    kpi(&mut snapshot, "kpi-2", "product-2");
    observation(&mut snapshot, "obs-2", "kpi-2", NOW - DAY);

    let lens = compose_executive_lens(&snapshot);

    assert_eq!(point(&lens.points, "product-1").observability.observed, 0);
    assert_eq!(point(&lens.points, "product-2").observability.observed, 1);
}

// ---------------------------------------------------------------------------
// Verified Evidence Coverage.
// ---------------------------------------------------------------------------

#[test]
fn coverage_counts_distinct_evidence_and_names_the_worst_state_by_the_frozen_order() {
    let mut snapshot = empty();
    product(&mut snapshot, "product-1", "One");
    evidence(
        &mut snapshot,
        "evidence-verified",
        "product-1",
        EvidenceVerification::Verified {
            verified_at: at(NOW - DAY),
            integrity_digest: digest(),
        },
    );
    evidence(
        &mut snapshot,
        "evidence-unverified",
        "product-1",
        EvidenceVerification::Unverified,
    );
    evidence(
        &mut snapshot,
        "evidence-mismatch",
        "product-1",
        EvidenceVerification::IntegrityMismatch,
    );
    evidence(
        &mut snapshot,
        "evidence-degraded",
        "product-1",
        EvidenceVerification::DegradedLastVerified {
            last_verified_at: at(NOW - 3 * DAY),
            integrity_digest: digest(),
        },
    );

    let one = &compose_executive_lens(&snapshot).points[0];

    assert_eq!((one.coverage.verified, one.coverage.linked), (1, 4));
    // A contradiction outranks an absence, which outranks a lapse.
    assert_eq!(one.coverage.worst, Some("integrity_mismatch"));
    assert_eq!(
        one.coverage.by_state,
        vec![
            ("integrity_mismatch", 1),
            ("unverified", 1),
            ("degraded_last_verified", 1),
            ("verified", 1),
        ]
    );
}

#[test]
fn an_unverified_state_outranks_a_readable_but_unpinned_one() {
    let mut snapshot = empty();
    product(&mut snapshot, "product-1", "One");
    evidence(
        &mut snapshot,
        "evidence-unpinned",
        "product-1",
        EvidenceVerification::ObservedUnpinned {
            observed_at: at(NOW - DAY),
            integrity_digest: digest(),
        },
    );
    evidence(
        &mut snapshot,
        "evidence-unverified",
        "product-1",
        EvidenceVerification::Unverified,
    );

    let one = &compose_executive_lens(&snapshot).points[0];

    assert_eq!(one.coverage.worst, Some("unverified"));
    assert_eq!(one.coverage.verified, 0);
}

#[test]
fn evidence_linked_to_another_target_is_not_coverage() {
    let mut snapshot = empty();
    product(&mut snapshot, "product-1", "One");
    evidence(
        &mut snapshot,
        "evidence-1",
        "product-other",
        EvidenceVerification::Unverified,
    );

    let one = &compose_executive_lens(&snapshot).points[0];

    assert_eq!(one.coverage.linked, 0);
    assert_eq!(one.coverage.worst, None);
}

// ---------------------------------------------------------------------------
// Quadrants, classification and order.
// ---------------------------------------------------------------------------

fn placed(due_at: i64, observed: bool) -> Option<Quadrant> {
    let mut snapshot = empty();
    product(&mut snapshot, "product-1", "One");
    project(&mut snapshot, "project-1", "product-1");
    milestone(
        &mut snapshot,
        "milestone-1",
        "project-1",
        due_at,
        DataClassification::Internal,
    );
    kpi(&mut snapshot, "kpi-1", "product-1");
    if observed {
        observation(&mut snapshot, "obs-1", "kpi-1", NOW - DAY);
    }
    compose_executive_lens(&snapshot).points[0].quadrant
}

#[test]
fn quadrants_keep_their_accepted_meanings_on_the_amended_axes() {
    let later = NOW + 60 * DAY;
    let passed = NOW - DAY;
    assert_eq!(placed(later, true), Some(Quadrant::KeepMomentum));
    assert_eq!(placed(passed, true), Some(Quadrant::MonitorClosely));
    assert_eq!(placed(later, false), Some(Quadrant::ExploreAndValidate));
    assert_eq!(placed(passed, false), Some(Quadrant::PrioritizeNow));
}

#[test]
fn the_classification_folds_upward_and_names_what_forced_it() {
    let mut snapshot = empty();
    product(&mut snapshot, "product-1", "One");
    project(&mut snapshot, "project-1", "product-1");
    milestone(
        &mut snapshot,
        "milestone-secret",
        "project-1",
        NOW + DAY,
        DataClassification::Confidential,
    );

    let one = &compose_executive_lens(&snapshot).points[0];

    assert_eq!(one.product_classification, DataClassification::Internal);
    assert_eq!(
        one.effective_classification,
        DataClassification::Confidential
    );
    let forced = one.classification_forced_by.as_ref().unwrap();
    assert_eq!(
        (forced.kind, forced.id.as_str()),
        ("milestone", "milestone-secret")
    );
}

#[test]
fn points_come_back_in_name_order_which_is_not_a_ranking() {
    let mut snapshot = empty();
    product(&mut snapshot, "product-z", "Zephyr");
    product(&mut snapshot, "product-a", "Aurora");
    product(&mut snapshot, "product-m", "Meridian");

    let lens = compose_executive_lens(&snapshot);
    let names: Vec<&str> = lens
        .points
        .iter()
        .map(|point| point.product_name.as_str())
        .collect();

    assert_eq!(names, vec!["Aurora", "Meridian", "Zephyr"]);
    assert_eq!(lens.ledger_revision, 11);
    assert_eq!(lens.as_of, at(NOW));
}

#[test]
fn an_evidence_link_is_named_by_its_evidence_and_target_and_claims_no_version() {
    let mut snapshot = empty();
    product(&mut snapshot, "product-1", "One");
    evidence(
        &mut snapshot,
        "evidence-1",
        "product-1",
        EvidenceVerification::Unverified,
    );

    let one = &compose_executive_lens(&snapshot).points[0];
    let link = one
        .coverage
        .contributions
        .iter()
        .find(|contribution| contribution.kind == "evidence_link")
        .unwrap();

    assert!(link.id.contains("evidence-1") && link.id.contains("product-1"));
    assert_eq!(link.version, None);
}
