//! The first production Tauri IPC command surface. Deliberately minimal: one
//! read-only Ledger status query, proving the full invoke -> command ->
//! managed-state -> response pipe works end to end before any real feature
//! route (still gated behind the app shell and its own execution authorization)
//! is built on top of it.
//!
//! `SafeErrorDto` covers only what `LedgerOpenError` needs today. The full
//! DG3 safe-error envelope (`messageKey`, typed `messageParams`,
//! `privateDetailRef`) is deliberately deferred to whichever slice first
//! wires a real `DomainError`-returning command -- inventing that shape now,
//! with no consumer, would be exactly the premature abstraction this
//! project's own conventions warn against.

// The safe error envelope is every command's Err type by contract (DG3);
// its size is the contract's, not an accident.
#![allow(clippy::result_large_err)]

use pmc_application::attention_ranking::{canonical_id_of, RankedAttentionItem};
use pmc_application::cockpit_adapter::compose_cockpit_from_snapshots;
use pmc_application::cockpit_aggregation::NoApprovedPeriodPort;
use pmc_application::executive_lens::{
    compose_executive_lens, Contribution, ExecutiveLens, LensPoint, TimingState,
    DUE_SOON_WINDOW_MILLIS,
};
use pmc_application::people_adapter::directory_entries_from_snapshot;
use pmc_application::people_composition::{compose_people_directory, RelationshipPurpose};
use pmc_application::product_adapter::product_detail_facts_from_snapshots;
use pmc_application::product_composition::{
    compose_product_detail, flagged_carried_work, health_reason_subject,
};
use pmc_application::route_composition::{
    ComposedEntityKind, ImpactJudgment, PageState, PeriodChange, RouteState,
};
use pmc_application::work_queue_adapter::work_item_facts_from_snapshots;
use pmc_application::work_queue_composition::{
    compose_work_queue, GetWorkQueue, WorkItemFacts, WorkItemKind, WorkQueueFilter,
};
use pmc_domain::attention::{AttentionThresholds, Freshness};
use pmc_domain::composition_source::LedgerSnapshotForCompositionPort;
use pmc_domain::projection_source::LedgerSnapshotForProjectionPort;
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::EvidenceVerification;
use serde::Serialize;
use tauri::State;

use crate::ledger_state::LedgerState;
use crate::runtime::host_correlation;
use crate::safe_error::SafeErrorDto;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerStatusDto {
    pub schema_version: u32,
    pub revision: u64,
}

/// Read-only: current schema version and commit revision of the real,
/// on-disk Product Ledger opened at startup.
#[tauri::command]
pub fn get_ledger_status(state: State<'_, LedgerState>) -> Result<LedgerStatusDto, SafeErrorDto> {
    let ledger = state
        .read()
        .map_err(|closed| closed.to_safe_error(&host_correlation()))?;
    let revision = ledger
        .revision()
        .map_err(|error| SafeErrorDto::from_open(error, &host_correlation()))?;
    Ok(LedgerStatusDto {
        schema_version: ledger.schema_version(),
        revision,
    })
}

// ---------------------------------------------------------------------------
// The Executive Cockpit route.
//
// The DTOs below are the transport edge, not the composition contract. They
// carry what the frozen DG3 query contract requires -- identity, authoritative
// revision and `as_of`, classification, freshness, attention, and the
// inspectable definition behind every count -- and deliberately no SQLite row,
// filesystem path, shell string or private diagnostic, which DG3 forbids
// surfacing.
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CountDto {
    pub count: usize,
    /// What exactly was counted. DG1 requires inspectable definitions: a bare
    /// number invites the reader to guess a denominator, which is how a count
    /// becomes a fabricated progress claim.
    pub definition: String,
    pub owner: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PulseDto {
    pub milestones: CountDto,
    pub commitments: CountDto,
    pub kpis: CountDto,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductDto {
    pub id: String,
    pub classification: String,
    pub revision: u64,
    pub owner: String,
    pub degraded: bool,
    pub attention_count: usize,
}

/// One Portfolio exception: what needs attention, not only why.
///
/// The record fields (`kind`, `id`, `label`) are what the DG3 S01 amendment §8
/// adds. Before it, the Cockpit could state a reason but never name the record
/// it was about, which left a reader to go and find it in the Work Queue.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExceptionDto {
    /// The lifecycle type of the record the flag was raised against.
    pub kind: String,
    pub id: String,
    /// The same label the Work Queue shows for that record: its title where
    /// the Ledger carries one, and its type and identifier where it does not.
    pub label: String,
    /// The reason identifier, stable across rewording of `explanation`.
    pub reason: String,
    pub explanation: String,
    /// The tier identifier, and why that tier outranks the ones below it.
    pub tier: String,
    pub tier_why: String,
    /// The deadline the flag is about, or `null` when it is not about a time.
    pub relevant_at_millis: Option<i64>,
    /// Why this item is ranked where it is, generated from the ordering
    /// itself so the stated reason cannot drift from the applied one.
    pub rank_rationale: String,
    pub classification: String,
    pub freshness: String,
    pub degraded: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutiveCockpitDto {
    pub state: String,
    pub as_of_millis: i64,
    pub ledger_revision: u64,
    /// False when no review period has been approved. The reason is carried
    /// alongside rather than shown as a zero delta, which would claim
    /// "nothing changed" instead of "there is nothing to compare against".
    pub period_comparable: bool,
    pub period_note: String,
    /// `null` only when the state is `outOfSync`: the two Ledger reads
    /// disagreed, so no count can be stated as true at one revision.
    pub pulse: Option<PulseDto>,
    pub products: Vec<ProductDto>,
    pub exceptions: Vec<ExceptionDto>,
    pub leader_conclusion: String,
    pub leader_intervention: Option<String>,
    /// The Executive Lens on the amended S01 axes. `null` only when the state
    /// is `outOfSync`.
    pub lens: Option<ExecutiveLensDto>,
}

/// One record behind a Lens measure. `version` is `null` for an Evidence
/// link, which has no identity or version of its own (S01 amendment §6).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LensContributionDto {
    pub kind: String,
    pub id: String,
    pub version: Option<u64>,
    pub classification: String,
    pub role: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LensTimingDto {
    /// `unknown`, `later`, `dueSoon` or `datePassed`.
    pub state: String,
    /// Whether the state is the high-exposure half, decided on the host.
    /// `None` when the state is `unknown`: Unknown is neither half.
    pub high: Option<bool>,
    pub earliest_due_at_millis: Option<i64>,
    pub milestone_count: usize,
    pub contributions: Vec<LensContributionDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LensObservabilityDto {
    pub observed: usize,
    /// Zero means Unknown, never 0%.
    pub defined: usize,
    pub known: bool,
    /// `None` when not `known`, for the same reason as timing.
    pub high: Option<bool>,
    pub latest_observed_at_millis: Option<i64>,
    pub contributions: Vec<LensContributionDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LensStateCountDto {
    pub state: String,
    pub count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LensCoverageDto {
    pub verified: usize,
    /// Zero means "no linked Evidence", never 0%.
    pub linked: usize,
    /// In the amendment's frozen severity order, most severe first.
    pub by_state: Vec<LensStateCountDto>,
    pub worst: Option<String>,
    pub contributions: Vec<LensContributionDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LensPointDto {
    pub product_id: String,
    pub product_name: String,
    pub product_version: u64,
    pub product_classification: String,
    pub timing: LensTimingDto,
    pub observability: LensObservabilityDto,
    pub coverage: LensCoverageDto,
    /// `null` whenever either axis is Unknown.
    pub quadrant: Option<String>,
    pub effective_classification: String,
    pub classification_forced_by: Option<LensContributionDto>,
    pub shared_project_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutiveLensDto {
    pub as_of_millis: i64,
    pub ledger_revision: u64,
    pub due_soon_window_millis: i64,
    /// In Product name order: navigation, not priority.
    pub points: Vec<LensPointDto>,
}

fn contribution_dto(contribution: &Contribution) -> LensContributionDto {
    LensContributionDto {
        kind: contribution.kind.to_owned(),
        id: contribution.id.clone(),
        version: contribution.version,
        classification: contribution.classification.as_persisted().to_owned(),
        role: contribution.role.to_owned(),
    }
}

fn contributions_dto(contributions: &[Contribution]) -> Vec<LensContributionDto> {
    contributions.iter().map(contribution_dto).collect()
}

/// The Lens as the webview receives it. Every judgement -- state, high or
/// low, quadrant, fold -- is made here, so the webview only renders.
fn lens_dto(lens: &ExecutiveLens) -> ExecutiveLensDto {
    ExecutiveLensDto {
        as_of_millis: lens.as_of.unix_millis(),
        ledger_revision: lens.ledger_revision,
        due_soon_window_millis: lens.due_soon_window_millis,
        points: lens.points.iter().map(lens_point_dto).collect(),
    }
}

fn lens_point_dto(point: &LensPoint) -> LensPointDto {
    LensPointDto {
        product_id: point.product_id.clone(),
        product_name: point.product_name.clone(),
        product_version: point.product_version,
        product_classification: point.product_classification.as_persisted().to_owned(),
        timing: LensTimingDto {
            state: point.timing.state.as_str().to_owned(),
            high: (point.timing.state != TimingState::Unknown)
                .then(|| point.timing.state.is_high()),
            earliest_due_at_millis: point.timing.earliest_due_at.map(UtcTimestamp::unix_millis),
            milestone_count: point.timing.milestone_count,
            contributions: contributions_dto(&point.timing.contributions),
        },
        observability: LensObservabilityDto {
            observed: point.observability.observed,
            defined: point.observability.defined,
            known: point.observability.is_known(),
            high: point
                .observability
                .is_known()
                .then(|| point.observability.is_high()),
            latest_observed_at_millis: point
                .observability
                .latest_observed_at
                .map(UtcTimestamp::unix_millis),
            contributions: contributions_dto(&point.observability.contributions),
        },
        coverage: LensCoverageDto {
            verified: point.coverage.verified,
            linked: point.coverage.linked,
            by_state: point
                .coverage
                .by_state
                .iter()
                .map(|(state, count)| LensStateCountDto {
                    state: (*state).to_owned(),
                    count: *count,
                })
                .collect(),
            worst: point.coverage.worst.map(str::to_owned),
            contributions: contributions_dto(&point.coverage.contributions),
        },
        quadrant: point.quadrant.map(|quadrant| quadrant.as_str().to_owned()),
        effective_classification: point.effective_classification.as_persisted().to_owned(),
        classification_forced_by: point
            .classification_forced_by
            .as_ref()
            .map(contribution_dto),
        shared_project_ids: point.shared_project_ids.clone(),
    }
}

/// Freshness as a stable identifier.
///
/// Surfaced beside every flag because the accepted ranking policy requires a
/// stale item to be shown with its uncertainty attached rather than silently
/// placed as though its facts were current.
const fn freshness_name(freshness: Freshness) -> &'static str {
    match freshness {
        Freshness::Fresh => "fresh",
        Freshness::Stale => "stale",
        Freshness::Unknown => "unknown",
    }
}

const fn route_state_name(state: RouteState) -> &'static str {
    match state {
        RouteState::Loading => "loading",
        RouteState::Empty => "empty",
        RouteState::Success => "success",
        RouteState::Stale => "stale",
        RouteState::Degraded => "degraded",
        RouteState::Error => "error",
        RouteState::PartialSuccess => "partialSuccess",
        RouteState::Cancelling => "cancelling",
        RouteState::Cancelled => "cancelled",
        RouteState::ApprovalRequired => "approvalRequired",
        RouteState::ClassificationDenied => "classificationDenied",
        RouteState::BackupDue => "backupDue",
        RouteState::EvidenceVerificationPending => "evidenceVerificationPending",
        RouteState::OutOfSync => "outOfSync",
    }
}

/// One exception as the Cockpit presents it, named by the same label the Work
/// Queue gives its record.
fn exception_dto(item: &RankedAttentionItem, records: &[WorkItemFacts]) -> ExceptionDto {
    let kind = WorkItemKind::of_target(&item.flag.target);
    let id = canonical_id_of(&item.flag.target).to_owned();
    let label = records
        .iter()
        .find(|record| record.kind == kind && record.id == id)
        .map_or_else(|| id.clone(), |record| record.label.clone());
    ExceptionDto {
        kind: kind.as_str().to_owned(),
        label,
        id,
        reason: item.flag.reason.as_str().to_owned(),
        explanation: item.flag.explanation.to_owned(),
        tier: item.tier.as_str().to_owned(),
        tier_why: item.tier.why().to_owned(),
        relevant_at_millis: item.relevant_at.map(UtcTimestamp::unix_millis),
        rank_rationale: item.rank_rationale.clone(),
        classification: item.flag.metadata.classification.as_persisted().to_owned(),
        freshness: freshness_name(item.flag.metadata.freshness).to_owned(),
        degraded: item.flag.metadata.degraded,
    }
}

/// Read-only: the composed Executive Cockpit for the real, on-disk Product
/// Ledger opened at startup.
///
/// A query in the strict sense -- it derives attention, ranks and composes,
/// and mutates nothing. `as_of` is supplied by this command rather than read
/// inside `pmc-ledger`, matching that crate's rule that it never originates a
/// timestamp.
///
/// Reads both Ledger snapshots, as the Work Queue does, because the
/// projection snapshot alone has no Action Requests, Decision Requests or
/// Issues: reading only it made the Cockpit say nothing needed attention while
/// the Work Queue listed overdue requests. The two reads must agree on the
/// Ledger revision; when they do not, the answer is `outOfSync` with nothing
/// in it rather than a blend of two moments (DG3 S01 amendment §8).
#[tauri::command]
pub fn get_executive_cockpit(
    state: State<'_, LedgerState>,
    as_of_millis: i64,
) -> Result<ExecutiveCockpitDto, SafeErrorDto> {
    let ledger = state
        .read()
        .map_err(|closed| closed.to_safe_error(&host_correlation()))?;
    let as_of = UtcTimestamp::from_unix_millis(as_of_millis);
    let unavailable = || {
        SafeErrorDto::host(
            "COCKPIT_SNAPSHOT_UNAVAILABLE",
            "desktop.snapshot_unavailable",
            &host_correlation(),
            true,
        )
    };
    let projection = ledger
        .read_projection_snapshot(as_of)
        .map_err(|_| unavailable())?;
    let composition = ledger
        .read_composition_snapshot(as_of)
        .map_err(|_| unavailable())?;

    if projection.ledger_revision != composition.ledger_revision {
        return Ok(ExecutiveCockpitDto {
            state: route_state_name(RouteState::OutOfSync).to_owned(),
            as_of_millis,
            // The older of the two, so the reported revision is one the whole
            // (empty) body is true at.
            ledger_revision: projection.ledger_revision.min(composition.ledger_revision),
            period_comparable: false,
            period_note: String::new(),
            pulse: None,
            products: Vec::new(),
            exceptions: Vec::new(),
            leader_conclusion: String::new(),
            leader_intervention: None,
            lens: None,
        });
    }

    let thresholds = AttentionThresholds::default();
    let composed = compose_cockpit_from_snapshots(
        &projection,
        &composition,
        thresholds,
        &NoApprovedPeriodPort,
    );
    let records = work_item_facts_from_snapshots(&projection, &composition, thresholds);
    let (period_comparable, period_note) = match &composed.body.period_change {
        PeriodChange::NotComparable { because } => (false, (*because).to_owned()),
        PeriodChange::Comparable { period_id, .. } => {
            (true, format!("compared against period {period_id}"))
        }
    };
    let pulse = &composed.body.pulse;
    Ok(ExecutiveCockpitDto {
        state: route_state_name(composed.state).to_owned(),
        as_of_millis: composed.as_of.unix_millis(),
        ledger_revision: composed.ledger_revision,
        period_comparable,
        period_note,
        pulse: Some(PulseDto {
            milestones: CountDto {
                count: pulse.milestones.count(),
                definition: pulse.milestones.definition().to_owned(),
                owner: pulse.milestones.provenance().owner().as_str().to_owned(),
            },
            commitments: CountDto {
                count: pulse.commitments.count(),
                definition: pulse.commitments.definition().to_owned(),
                owner: pulse.commitments.provenance().owner().as_str().to_owned(),
            },
            kpis: CountDto {
                count: pulse.kpis.count(),
                definition: pulse.kpis.definition().to_owned(),
                owner: pulse.kpis.provenance().owner().as_str().to_owned(),
            },
        }),
        products: composed
            .body
            .products
            .iter()
            .map(|product| ProductDto {
                id: product.id.clone(),
                classification: product.classification.as_persisted().to_owned(),
                revision: product.revision,
                owner: product.owner.as_str().to_owned(),
                degraded: product.degraded,
                attention_count: product.attention.len(),
            })
            .collect(),
        exceptions: composed
            .body
            .exceptions
            .iter()
            .map(|item| exception_dto(item, &records))
            .collect(),
        leader_conclusion: composed.body.leader_briefing.conclusion.clone(),
        leader_intervention: composed.body.leader_briefing.intervention.clone(),
        lens: Some(lens_dto(&compose_executive_lens(&composition))),
    })
}

/// One Portfolio row: the Product's Lens measures, and how much flagged work
/// the people accountable for it carry.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioRowDto {
    pub point: LensPointDto,
    /// Distinct work items with at least one flag, across everyone
    /// accountable for the Product. Carried by the people, never the
    /// Product's own work (DG3 O01 amendment).
    pub flagged_work_count: usize,
    /// The point's classification folded with every counted item's, so the
    /// count cannot reveal more than the row is labelled.
    pub classification: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioOverviewDto {
    pub state: String,
    pub as_of_millis: i64,
    pub ledger_revision: u64,
    pub due_soon_window_millis: i64,
    /// In Product name order: navigation, not priority.
    pub rows: Vec<PortfolioRowDto>,
    pub offset: usize,
    pub limit: usize,
    pub total: usize,
    pub has_more: bool,
}

/// Read-only: the Portfolio overview for the real, on-disk Product Ledger.
///
/// Reads both snapshots, as the Cockpit and the Work Queue do, and reports
/// `outOfSync` with no rows when a write landed between them. Rows are the
/// Executive Lens points in name order -- the same measures the Cockpit
/// shows, so the two routes cannot disagree about a Product.
#[tauri::command]
pub fn get_portfolio_overview(
    state: State<'_, LedgerState>,
    as_of_millis: i64,
    offset: usize,
    limit: usize,
) -> Result<PortfolioOverviewDto, SafeErrorDto> {
    let ledger = state
        .read()
        .map_err(|closed| closed.to_safe_error(&host_correlation()))?;
    let as_of = UtcTimestamp::from_unix_millis(as_of_millis);
    let unavailable = || {
        SafeErrorDto::host(
            "PORTFOLIO_SNAPSHOT_UNAVAILABLE",
            "desktop.snapshot_unavailable",
            &host_correlation(),
            true,
        )
    };
    let projection = ledger
        .read_projection_snapshot(as_of)
        .map_err(|_| unavailable())?;
    let composition = ledger
        .read_composition_snapshot(as_of)
        .map_err(|_| unavailable())?;
    if projection.ledger_revision != composition.ledger_revision {
        return Ok(PortfolioOverviewDto {
            state: route_state_name(RouteState::OutOfSync).to_owned(),
            as_of_millis,
            ledger_revision: projection.ledger_revision.min(composition.ledger_revision),
            due_soon_window_millis: DUE_SOON_WINDOW_MILLIS,
            rows: Vec::new(),
            offset,
            limit,
            total: 0,
            has_more: false,
        });
    }

    let lens = compose_executive_lens(&composition);
    let total = lens.points.len();
    let page = PageState {
        offset,
        limit,
        total,
    };
    let thresholds = AttentionThresholds::default();
    let rows = lens
        .points
        .iter()
        .skip(offset)
        .take(limit)
        .map(|point| {
            let (count, carried) = product_detail_facts_from_snapshots(
                &point.product_id,
                &projection,
                &composition,
                thresholds,
            )
            .map_or((0, None), |facts| flagged_carried_work(&facts));
            let classification = carried.map_or(point.effective_classification, |carried| {
                point.effective_classification.combine(carried)
            });
            PortfolioRowDto {
                point: lens_point_dto(point),
                flagged_work_count: count,
                classification: classification.as_persisted().to_owned(),
            }
        })
        .collect();
    Ok(PortfolioOverviewDto {
        state: route_state_name(if total == 0 {
            RouteState::Empty
        } else {
            RouteState::Success
        })
        .to_owned(),
        as_of_millis: composition.ledger_as_of_utc.unix_millis(),
        ledger_revision: composition.ledger_revision,
        due_soon_window_millis: lens.due_soon_window_millis,
        rows,
        offset: page.offset,
        limit: page.limit,
        total: page.total,
        has_more: page.has_more(),
    })
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonDto {
    pub id: String,
    /// The effective classification: the most restrictive of the person and
    /// everything the entry exposes about them, never the record's own weaker
    /// label. Composition folds it upward so a directory cannot leak by
    /// aggregation.
    pub classification: String,
    pub revision: u64,
    pub owner: String,
    pub responsibility_count: usize,
    pub dependency_count: usize,
    pub outstanding_request_count: usize,
    pub display_name: String,
    /// `person` or `organization`.
    pub kind: String,
    /// What the person is responsible for or depends on, by name. Their
    /// classifications are already folded into `classification`.
    pub relationships: Vec<PersonRelationshipDto>,
    /// Action Requests still waiting on this person, by title.
    pub outstanding_requests: Vec<PersonRequestDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonRelationshipDto {
    pub subject_id: String,
    pub subject_label: String,
    /// `responsibility` or `dependency`.
    pub purpose: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonRequestDto {
    pub id: String,
    /// The request's title where the snapshot carries it; the identifier
    /// otherwise, never an invented name.
    pub label: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeopleDirectoryDto {
    pub state: String,
    pub as_of_millis: i64,
    pub ledger_revision: u64,
    pub people: Vec<PersonDto>,
    pub offset: usize,
    pub limit: usize,
    pub total: usize,
    pub has_more: bool,
}

/// Read-only: the People directory for the real, on-disk Product Ledger.
///
/// Composition folds classification upward, so an entry is presented at the
/// most restrictive classification of anything it exposes.
#[tauri::command]
pub fn get_people_directory(
    state: State<'_, LedgerState>,
    as_of_millis: i64,
    offset: usize,
    limit: usize,
) -> Result<PeopleDirectoryDto, SafeErrorDto> {
    let ledger = state
        .read()
        .map_err(|closed| closed.to_safe_error(&host_correlation()))?;
    let snapshot = ledger
        .read_composition_snapshot(UtcTimestamp::from_unix_millis(as_of_millis))
        .map_err(|_| {
            SafeErrorDto::host(
                "PEOPLE_SNAPSHOT_UNAVAILABLE",
                "desktop.snapshot_unavailable",
                &host_correlation(),
                true,
            )
        })?;
    let entries = directory_entries_from_snapshot(&snapshot);
    let total = entries.len();
    // Counted before paging: the reader is told how many people exist, not
    // how many fit on this page.
    let counts: Vec<(usize, usize, usize)> = entries
        .iter()
        .map(|entry| {
            let responsibilities = entry
                .relationships
                .iter()
                .filter(|relationship| relationship.purpose == RelationshipPurpose::Responsibility)
                .count();
            (
                responsibilities,
                entry.relationships.len() - responsibilities,
                entry.requests.len(),
            )
        })
        .collect();
    let composed = compose_people_directory(
        &entries,
        UtcTimestamp::from_unix_millis(as_of_millis),
        snapshot.ledger_revision,
        PageState {
            offset,
            limit,
            total,
        },
    );
    let people = composed
        .body
        .stakeholders
        .iter()
        .enumerate()
        .map(|(index, person)| {
            let (responsibilities, dependencies, requests) =
                counts.get(offset + index).copied().unwrap_or((0, 0, 0));
            let entry = entries
                .iter()
                .find(|entry| entry.stakeholder.id == person.id);
            PersonDto {
                id: person.id.clone(),
                classification: person.classification.as_persisted().to_owned(),
                revision: person.revision,
                owner: person.owner.as_str().to_owned(),
                responsibility_count: responsibilities,
                dependency_count: dependencies,
                outstanding_request_count: requests,
                display_name: entry.map_or_else(
                    || person.id.clone(),
                    |entry| entry.stakeholder.display_name.clone(),
                ),
                kind: snapshot
                    .stakeholders
                    .iter()
                    .find(|stakeholder| stakeholder.id.as_str() == person.id)
                    .map_or("person", |stakeholder| stakeholder.kind.as_persisted())
                    .to_owned(),
                relationships: entry.map_or_else(Vec::new, |entry| {
                    entry
                        .relationships
                        .iter()
                        .map(|relationship| PersonRelationshipDto {
                            subject_id: relationship.subject_id.clone(),
                            subject_label: relationship.subject_label.clone(),
                            purpose: relationship.purpose.as_str().to_owned(),
                        })
                        .collect()
                }),
                outstanding_requests: entry.map_or_else(Vec::new, |entry| {
                    entry
                        .requests
                        .iter()
                        .map(|request| PersonRequestDto {
                            id: request.id.clone(),
                            label: snapshot
                                .action_requests
                                .iter()
                                .find(|record| record.id == request.id)
                                .map_or_else(|| request.id.clone(), |record| record.title.clone()),
                        })
                        .collect()
                }),
            }
        })
        .collect();
    Ok(PeopleDirectoryDto {
        state: route_state_name(composed.state).to_owned(),
        as_of_millis: composed.as_of.unix_millis(),
        ledger_revision: composed.ledger_revision,
        people,
        offset: composed.body.page.offset,
        limit: composed.body.page.limit,
        total: composed.body.page.total,
        has_more: composed.body.page.has_more(),
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkQueueItemDto {
    /// Which lifecycle this item belongs to. Required on every item: the
    /// Work Queue must keep the five types separate, and a list without a
    /// discriminant would collapse them while looking correct.
    pub kind: String,
    pub id: String,
    pub label: String,
    pub state_label: String,
    pub classification: String,
    pub revision: u64,
    pub owner: String,
    /// When the Ledger row behind this item was read.
    ///
    /// Per item, not per response, because the five kinds come from TWO
    /// snapshots read in two separate transactions: Actions and Risks from
    /// the projection, the other three from the composition. One envelope
    /// timestamp cannot describe both, and the envelope `asOfMillis` is the
    /// caller request time rather than a read time, so without this field
    /// nothing on the surface answers "when was this actually read".
    ///
    /// This is the fourth element of the provenance every composed route
    /// requires -- owner, source identity, authoritative revision, `as_of` --
    /// and the Work Queue (S03) must preserve provenance. The other three are
    /// `owner`, `id` and `revision` above.
    ///
    /// Item-level freshness is deliberately NOT surfaced beside it. The
    /// adapter reads each snapshot in one transaction and can only ever
    /// report `Fresh`, so a per-item freshness field would be a value no code
    /// path can vary -- a shape that looks like information and carries none.
    /// Freshness is carried per attention flag, where it does vary.
    pub as_of_millis: i64,
    /// What the lifecycle admits. Not permissions: preparation,
    /// classification, policy and approval are all further gates, so a
    /// surface may present these as not-excluded-by-lifecycle and never as
    /// what may be done now.
    pub lifecycle_legal_intents: Vec<String>,
    /// `null` where the Ledger holds no deadline for this kind at all. Never
    /// substituted with a default.
    pub relevant_at_millis: Option<i64>,
    /// When the work an Action Request asks for is promised to be done;
    /// `null` for every other kind and for a request that names no date.
    /// Not a deadline of the request itself, so it never ranks the item.
    pub promised_at_millis: Option<i64>,
    pub placement: String,
    pub placement_rationale: String,
    /// Every flag raised against this record, ordered by the accepted ranking
    /// policy. The first is the one that placed the item.
    ///
    /// The Work Queue (S03) must preserve attention FLAGS, and DG0
    /// lists "attention reasons" as Work Queue content. A count satisfies
    /// neither: it tells a reader that two things are wrong without telling
    /// them what, which is exactly the shape that looks complete while
    /// carrying none of the required information. Empty is a fact -- nothing
    /// has flagged this record -- not a missing value.
    pub attention: Vec<WorkQueueAttentionDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkQueueAttentionDto {
    /// The reason identifier, not its `Debug` rendering and not its prose, so
    /// a surface can key off it without breaking when either changes.
    pub reason: String,
    /// The evaluator own explanation of what happened.
    pub explanation: String,
    /// The tier that placed this flag, and why that tier outranks the ones
    /// below it. Both come from the ordering itself, so the stated reason
    /// cannot drift from the reason actually used.
    pub tier: String,
    pub tier_why: String,
    /// The deadline this flag is about, or `null` when the reason is not
    /// about a time at all. Never substituted.
    pub relevant_at_millis: Option<i64>,
    pub rank_rationale: String,
    pub classification: String,
    pub freshness: String,
    /// Reported, never used to improve the item position: a stale fact is not
    /// a more certain one.
    pub degraded: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkQueueKindCountDto {
    pub kind: String,
    pub count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkQueueDto {
    pub state: String,
    pub as_of_millis: i64,
    pub ledger_revision: u64,
    pub items: Vec<WorkQueueItemDto>,
    /// Every kind, including those with none, so zero and absent stay
    /// distinguishable.
    pub counts_by_kind: Vec<WorkQueueKindCountDto>,
    pub offset: usize,
    pub limit: usize,
    pub total: usize,
    pub has_more: bool,
}

/// Read-only: the composed Work Queue for the real, on-disk Product Ledger.
///
/// The five lifecycle types live behind two ports -- Actions and Risks in the
/// projection snapshot, the other three in the composition snapshot -- and
/// those are two separate reads. If a write lands between them the two
/// snapshots describe different instants, and composing across them would
/// present two moments as one. That case returns `outOfSync` with no items
/// rather than a plausible-looking blend: DG3 defines that state for exactly
/// this, and a caller can retry.
#[tauri::command]
pub fn get_work_queue(
    state: State<'_, LedgerState>,
    as_of_millis: i64,
    offset: usize,
    limit: usize,
    kinds: Vec<String>,
    only_flagged: bool,
) -> Result<WorkQueueDto, SafeErrorDto> {
    let ledger = state
        .read()
        .map_err(|closed| closed.to_safe_error(&host_correlation()))?;
    let as_of = UtcTimestamp::from_unix_millis(as_of_millis);
    let projection = ledger.read_projection_snapshot(as_of).map_err(|_| {
        SafeErrorDto::host(
            "WORK_QUEUE_SNAPSHOT_UNAVAILABLE",
            "desktop.snapshot_unavailable",
            &host_correlation(),
            true,
        )
    })?;
    let composition = ledger.read_composition_snapshot(as_of).map_err(|_| {
        SafeErrorDto::host(
            "WORK_QUEUE_SNAPSHOT_UNAVAILABLE",
            "desktop.snapshot_unavailable",
            &host_correlation(),
            true,
        )
    })?;

    if projection.ledger_revision != composition.ledger_revision {
        return Ok(WorkQueueDto {
            state: route_state_name(RouteState::OutOfSync).to_owned(),
            as_of_millis,
            // The older of the two, so the reported revision is one the whole
            // (empty) body is true at rather than the newer half.
            ledger_revision: projection.ledger_revision.min(composition.ledger_revision),
            items: Vec::new(),
            counts_by_kind: Vec::new(),
            offset,
            limit,
            total: 0,
            has_more: false,
        });
    }

    let facts =
        work_item_facts_from_snapshots(&projection, &composition, AttentionThresholds::default());
    let requested_kinds = kinds
        .iter()
        .filter_map(|name| {
            WorkItemKind::ALL
                .into_iter()
                .find(|kind| kind.as_str() == name)
        })
        .collect();
    let composed = compose_work_queue(
        &GetWorkQueue {
            as_of,
            page: PageState {
                offset,
                limit,
                total: 0,
            },
            filter: WorkQueueFilter {
                kinds: requested_kinds,
                only_flagged,
            },
        },
        &facts,
        composition.ledger_revision,
    );

    Ok(WorkQueueDto {
        state: route_state_name(composed.state).to_owned(),
        as_of_millis: composed.as_of.unix_millis(),
        ledger_revision: composed.ledger_revision,
        items: composed
            .body
            .items
            .iter()
            .map(|item| WorkQueueItemDto {
                kind: item.kind.as_str().to_owned(),
                id: item.id.clone(),
                label: item.label.clone(),
                state_label: item.state_label.to_owned(),
                classification: item.classification.as_persisted().to_owned(),
                revision: item.revision,
                owner: item.kind.owner().as_str().to_owned(),
                as_of_millis: item.as_of.unix_millis(),
                lifecycle_legal_intents: item
                    .lifecycle_legal_intents
                    .iter()
                    .map(|intent| (*intent).to_owned())
                    .collect(),
                relevant_at_millis: item.relevant_at.map(UtcTimestamp::unix_millis),
                promised_at_millis: item.promised_at.map(UtcTimestamp::unix_millis),
                placement: item.placement.as_str().to_owned(),
                placement_rationale: item.placement_rationale.clone(),
                attention: item.attention.iter().map(attention_dto).collect(),
            })
            .collect(),
        counts_by_kind: composed
            .body
            .counts_by_kind
            .iter()
            .map(|entry| WorkQueueKindCountDto {
                kind: entry.kind.as_str().to_owned(),
                count: entry.count,
            })
            .collect(),
        offset: composed.body.page.offset,
        limit: composed.body.page.limit,
        total: composed.body.page.total,
        has_more: composed.body.page.has_more(),
    })
}

/// One attention flag as every route presents it. One mapping, so the Work
/// Queue and the Product inspector cannot describe the same flag differently.
fn attention_dto(ranked: &RankedAttentionItem) -> WorkQueueAttentionDto {
    WorkQueueAttentionDto {
        reason: ranked.flag.reason.as_str().to_owned(),
        explanation: ranked.flag.explanation.to_owned(),
        tier: ranked.tier.as_str().to_owned(),
        tier_why: ranked.tier.why().to_owned(),
        relevant_at_millis: ranked.relevant_at.map(UtcTimestamp::unix_millis),
        rank_rationale: ranked.rank_rationale.clone(),
        classification: ranked
            .flag
            .metadata
            .classification
            .as_persisted()
            .to_owned(),
        freshness: freshness_name(ranked.flag.metadata.freshness).to_owned(),
        degraded: ranked.flag.metadata.degraded,
    }
}

/// A stable name for every kind a route can compose. Exhaustive, so a new
/// kind cannot reach a surface as an empty string.
const fn entity_kind_name(kind: ComposedEntityKind) -> &'static str {
    match kind {
        ComposedEntityKind::Product => "product",
        ComposedEntityKind::Initiative => "initiative",
        ComposedEntityKind::Project => "project",
        ComposedEntityKind::Milestone => "milestone",
        ComposedEntityKind::Stakeholder => "stakeholder",
        ComposedEntityKind::Kpi => "kpi_definition",
        ComposedEntityKind::Roadmap => "roadmap",
        ComposedEntityKind::Evidence => "evidence_reference",
        ComposedEntityKind::ActionRequest => "action_request",
        ComposedEntityKind::Action => "action",
        ComposedEntityKind::DecisionRequest => "decision_request",
        ComposedEntityKind::Risk => "risk",
        ComposedEntityKind::Issue => "issue",
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductSummaryDto {
    pub id: String,
    pub label: String,
    /// The **effective** classification: the most restrictive of the Product
    /// and everything the inspector exposes. See `classificationForcedBy`.
    pub classification: String,
    pub revision: u64,
    pub owner: String,
    pub as_of_millis: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassificationFoldDto {
    pub forced_by_kind: String,
    pub forced_by_id: String,
    pub classification: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StructureEntryDto {
    pub kind: String,
    pub id: String,
    pub label: String,
    pub classification: String,
    pub revision: u64,
    /// `Some` only for an Initiative: the Project it was reached through.
    /// The surface must present that as an association, never containment.
    pub via: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceEntryDto {
    pub id: String,
    pub role: Option<String>,
    /// verified | degraded_last_verified | observed_unpinned | unverified |
    /// integrity_mismatch
    pub verification: String,
    /// The observation time: verified / last verified / observed, by state.
    pub verified_at_millis: Option<i64>,
    /// Whether a fingerprint is pinned. Independent of `verification`.
    pub pinned: bool,
    /// The Evidence record's current classification.
    pub classification: String,
    /// The classification recorded when the link was made. A separate fact.
    pub classification_at_link: String,
    pub linked_at_millis: i64,
    pub revision: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CarriedWorkDto {
    pub kind: String,
    pub id: String,
    pub label: String,
    pub state_label: String,
    pub classification: String,
    pub revision: u64,
    pub as_of_millis: i64,
    /// What the lifecycle admits. Not permissions.
    pub lifecycle_legal_intents: Vec<String>,
    pub attention: Vec<WorkQueueAttentionDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountablePersonDto {
    pub id: String,
    pub display_name: String,
    pub purpose: String,
    pub classification: String,
    pub revision: u64,
    pub other_products_accountable_for: usize,
    /// Work this person owns. Carried by the person, never the Product's.
    pub carried: Vec<CarriedWorkDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthReasonDto {
    pub text: String,
    /// The verification kind or attention reason behind `text`; `None` when
    /// it could not be read back, in which case `text` is shown as is.
    pub reason_code: Option<String>,
    /// `evidence` or a Work Queue kind.
    pub subject_kind: Option<String>,
    pub subject_label: Option<String>,
    pub owner: String,
    pub source_record_id: String,
    pub source_field: String,
    pub source_revision: u64,
    pub as_of_millis: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductDetailDto {
    pub state: String,
    pub as_of_millis: i64,
    pub ledger_revision: u64,
    /// `None` only when `state` is `outOfSync`.
    pub product: Option<ProductSummaryDto>,
    pub classification_forced_by: Option<ClassificationFoldDto>,
    pub structure: Vec<StructureEntryDto>,
    pub evidence: Vec<EvidenceEntryDto>,
    pub people: Vec<AccountablePersonDto>,
    /// `發生`: current conditions, each attributed to what raised it.
    pub health_reasons: Vec<HealthReasonDto>,
    /// `影響`: "unassessed" until a person records a judgment.
    pub impact: String,
}

const fn impact_name(impact: ImpactJudgment) -> &'static str {
    match impact {
        ImpactJudgment::Unassessed => "unassessed",
    }
}

/// Read-only: the Product detail and O01 inspector for the real, on-disk
/// Product Ledger, per the DG3 O01 amendment of 2026-09-07.
///
/// Reads both snapshots. If a write lands between the two reads the
/// snapshots describe different instants; the command reports `outOfSync`
/// with no product rather than compose across them. A Product the Ledger
/// does not hold is an error, not an empty inspector: "nothing attached" and
/// "no such Product" are different answers.
#[tauri::command]
pub fn get_product_detail(
    state: State<'_, LedgerState>,
    as_of_millis: i64,
    product_id: String,
) -> Result<ProductDetailDto, SafeErrorDto> {
    let ledger = state
        .read()
        .map_err(|closed| closed.to_safe_error(&host_correlation()))?;
    let as_of = UtcTimestamp::from_unix_millis(as_of_millis);
    let projection = ledger.read_projection_snapshot(as_of).map_err(|_| {
        SafeErrorDto::host(
            "PRODUCT_DETAIL_SNAPSHOT_UNAVAILABLE",
            "desktop.snapshot_unavailable",
            &host_correlation(),
            true,
        )
    })?;
    let composition = ledger.read_composition_snapshot(as_of).map_err(|_| {
        SafeErrorDto::host(
            "PRODUCT_DETAIL_SNAPSHOT_UNAVAILABLE",
            "desktop.snapshot_unavailable",
            &host_correlation(),
            true,
        )
    })?;

    if projection.ledger_revision != composition.ledger_revision {
        return Ok(ProductDetailDto {
            state: route_state_name(RouteState::OutOfSync).to_owned(),
            as_of_millis,
            ledger_revision: projection.ledger_revision.min(composition.ledger_revision),
            product: None,
            classification_forced_by: None,
            structure: Vec::new(),
            evidence: Vec::new(),
            people: Vec::new(),
            health_reasons: Vec::new(),
            impact: impact_name(ImpactJudgment::Unassessed).to_owned(),
        });
    }

    let facts = product_detail_facts_from_snapshots(
        &product_id,
        &projection,
        &composition,
        AttentionThresholds::default(),
    )
    .ok_or(SafeErrorDto::host(
        "PRODUCT_NOT_FOUND",
        "desktop.product_not_found",
        &host_correlation(),
        false,
    ))?;
    let composed = compose_product_detail(&facts, composition.ledger_revision).map_err(|_| {
        SafeErrorDto::host(
            "PRODUCT_DETAIL_UNATTRIBUTABLE",
            "desktop.product_detail_unattributable",
            &host_correlation(),
            false,
        )
    })?;
    let body = composed.body;

    Ok(ProductDetailDto {
        state: route_state_name(composed.state).to_owned(),
        as_of_millis: composed.as_of.unix_millis(),
        ledger_revision: composed.ledger_revision,
        product: Some(ProductSummaryDto {
            id: body.product.id.clone(),
            label: body.product_label.clone(),
            classification: body.product.classification.as_persisted().to_owned(),
            revision: body.product.revision,
            owner: body.product.owner.as_str().to_owned(),
            as_of_millis: body.product.as_of.unix_millis(),
        }),
        classification_forced_by: body.classification_forced_by.as_ref().map(|fold| {
            ClassificationFoldDto {
                forced_by_kind: entity_kind_name(fold.forced_by_kind).to_owned(),
                forced_by_id: fold.forced_by_id.clone(),
                classification: fold.classification.as_persisted().to_owned(),
            }
        }),
        structure: body
            .structure
            .iter()
            .map(|entry| StructureEntryDto {
                kind: entity_kind_name(entry.entity.kind).to_owned(),
                id: entry.entity.id.clone(),
                label: entry.label.clone(),
                classification: entry.entity.classification.as_persisted().to_owned(),
                revision: entry.entity.revision,
                via: entry.via.clone(),
            })
            .collect(),
        evidence: body
            .evidence
            .iter()
            .map(|entry| {
                let verified_at_millis = match &entry.verification {
                    EvidenceVerification::Verified { verified_at, .. } => {
                        Some(verified_at.unix_millis())
                    }
                    EvidenceVerification::ObservedUnpinned { observed_at, .. } => {
                        Some(observed_at.unix_millis())
                    }
                    EvidenceVerification::DegradedLastVerified {
                        last_verified_at, ..
                    } => Some(last_verified_at.unix_millis()),
                    EvidenceVerification::Unverified | EvidenceVerification::IntegrityMismatch => {
                        None
                    }
                };
                EvidenceEntryDto {
                    id: entry.entity.id.clone(),
                    role: entry.role.map(|role| role.as_persisted().to_owned()),
                    verification: entry.verification.kind_as_persisted().to_owned(),
                    verified_at_millis,
                    pinned: entry.pinned,
                    classification: entry.entity.classification.as_persisted().to_owned(),
                    classification_at_link: entry.classification_at_link.as_persisted().to_owned(),
                    linked_at_millis: entry.linked_at.unix_millis(),
                    revision: entry.entity.revision,
                }
            })
            .collect(),
        people: body
            .people
            .iter()
            .map(|person| AccountablePersonDto {
                id: person.entity.id.clone(),
                display_name: person.display_name.clone(),
                purpose: person.purpose.as_str().to_owned(),
                classification: person.entity.classification.as_persisted().to_owned(),
                revision: person.entity.revision,
                other_products_accountable_for: person.other_products_accountable_for,
                carried: person
                    .carried
                    .iter()
                    .map(|item| CarriedWorkDto {
                        kind: item.kind.as_str().to_owned(),
                        id: item.id.clone(),
                        label: item.label.clone(),
                        state_label: item.state_label.to_owned(),
                        classification: item.classification.as_persisted().to_owned(),
                        revision: item.revision,
                        as_of_millis: item.as_of.unix_millis(),
                        lifecycle_legal_intents: item
                            .lifecycle_legal_intents
                            .iter()
                            .map(|intent| (*intent).to_owned())
                            .collect(),
                        attention: item.attention.iter().map(attention_dto).collect(),
                    })
                    .collect(),
            })
            .collect(),
        health_reasons: body
            .health_reasons
            .iter()
            .map(|reason| {
                let subject = health_reason_subject(&body, reason);
                let (reason_code, subject_kind, subject_label) =
                    subject.map_or((None, None, None), |subject| {
                        (
                            Some(subject.reason_code.to_owned()),
                            Some(subject.subject_kind.to_owned()),
                            Some(subject.subject_label),
                        )
                    });
                HealthReasonDto {
                    text: reason.value().clone(),
                    reason_code,
                    subject_kind,
                    subject_label,
                    owner: reason.provenance().owner().as_str().to_owned(),
                    source_record_id: reason.provenance().source_record_id().to_owned(),
                    source_field: reason.provenance().source_field().to_owned(),
                    source_revision: reason.provenance().source_revision(),
                    as_of_millis: reason.provenance().as_of().unix_millis(),
                }
            })
            .collect(),
        impact: impact_name(body.impact).to_owned(),
    })
}
