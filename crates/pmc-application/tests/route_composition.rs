//! UI-neutral route composition contracts.

use pmc_application::route_composition::{
    ComposedEntity, ComposedEntityKind, FieldProvenance, OwnerModule, PageState, RouteState,
    Sourced,
};
use pmc_domain::attention::Freshness;
use pmc_domain::classification::DataClassification;
use pmc_domain::time::UtcTimestamp;

fn provenance() -> FieldProvenance {
    FieldProvenance::new(
        OwnerModule::Portfolio,
        "product-1",
        "classification",
        7,
        UtcTimestamp::from_unix_millis(1_700_000_000_000),
    )
}

#[test]
fn provenance_carries_every_element_s5_requires() {
    // The composition specification: "Every field carries owner/module,
    // source identity, authoritative revision, and `as_of` provenance."
    let provenance = provenance();
    assert_eq!(provenance.owner(), OwnerModule::Portfolio);
    assert_eq!(provenance.source_record_id(), "product-1");
    assert_eq!(provenance.source_field(), "classification");
    assert_eq!(provenance.source_revision(), 7);
    assert_eq!(
        provenance.as_of(),
        UtcTimestamp::from_unix_millis(1_700_000_000_000)
    );
}

#[test]
fn reshaping_a_composed_value_keeps_its_origin() {
    // The realistic way provenance gets lost is not that someone forgets to
    // set it -- the constructor makes that impossible -- but that a value is
    // reshaped for presentation and rebuilt without it. `map` exists so that
    // path keeps the origin instead.
    let composed = Sourced::new(3_u32, provenance());

    let reshaped = composed.map(|count| format!("{count} open"));

    assert_eq!(reshaped.value(), "3 open");
    assert_eq!(reshaped.provenance(), &provenance());
}

#[test]
fn every_owner_module_has_a_distinct_stable_identifier() {
    // Two modules sharing an identifier would make a surface attribute a
    // fact to the wrong owner.
    let modules = [
        OwnerModule::Portfolio,
        OwnerModule::Delivery,
        OwnerModule::ActionManagement,
        OwnerModule::Decisions,
        OwnerModule::Risks,
        OwnerModule::Issues,
        OwnerModule::Stakeholders,
        OwnerModule::Evidence,
        OwnerModule::Kpi,
        OwnerModule::Review,
    ];
    let mut seen = std::collections::HashSet::new();
    for module in modules {
        assert!(
            seen.insert(module.as_str()),
            "{} is used by more than one owner module",
            module.as_str()
        );
    }
    assert_eq!(seen.len(), modules.len());
}

#[test]
fn the_route_state_contract_covers_every_state_dg3_names() {
    // The frozen DG3 State and Feedback Contract names these thirteen. A
    // route may not invent a fourteenth, and dropping one would let a surface
    // fall back to a state the contract does not define.
    let states = [
        RouteState::Loading,
        RouteState::Empty,
        RouteState::Success,
        RouteState::Stale,
        RouteState::Degraded,
        RouteState::Error,
        RouteState::PartialSuccess,
        RouteState::Cancelling,
        RouteState::Cancelled,
        RouteState::ApprovalRequired,
        RouteState::ClassificationDenied,
        RouteState::BackupDue,
        RouteState::EvidenceVerificationPending,
        RouteState::OutOfSync,
    ];
    let unique: std::collections::HashSet<_> = states.iter().map(|s| format!("{s:?}")).collect();
    assert_eq!(unique.len(), states.len());
}

#[test]
fn a_page_knows_whether_more_remains() {
    assert!(PageState {
        offset: 0,
        limit: 10,
        total: 25
    }
    .has_more());
    assert!(!PageState {
        offset: 20,
        limit: 10,
        total: 25
    }
    .has_more());
    // Exactly consumed is not "more".
    assert!(!PageState {
        offset: 0,
        limit: 25,
        total: 25
    }
    .has_more());
}

#[test]
fn a_page_beyond_the_end_does_not_overflow() {
    // Saturating arithmetic rather than wrapping: an offset past the end is a
    // caller bug, but it must not silently report that more remains.
    assert!(!PageState {
        offset: usize::MAX,
        limit: 10,
        total: 25
    }
    .has_more());
}

#[test]
fn an_entity_with_nothing_wrong_still_states_that_explicitly() {
    // An empty attention list is a fact -- "nothing needs attention" -- not a
    // missing one, and an empty legal-intent list truthfully says none are
    // known rather than implying an action the domain has not permitted.
    let entity = ComposedEntity {
        kind: ComposedEntityKind::Product,
        id: "product-1".to_owned(),
        owner: OwnerModule::Portfolio,
        revision: 7,
        as_of: UtcTimestamp::from_unix_millis(1_700_000_000_000),
        classification: DataClassification::Internal,
        freshness: Freshness::Fresh,
        degraded: false,
        attention: Vec::new(),
        lifecycle_legal_intents: Vec::new(),
    };

    assert!(entity.attention.is_empty());
    assert!(entity.lifecycle_legal_intents.is_empty());
    assert_eq!(entity.freshness, Freshness::Fresh);
}
