//! Product detail and Product-health inspector composition (S02 detail,
//! O01), per the accepted DG3 O01 amendment.
//!
//! The amendment's whole point is a framing: work is reached through the
//! people accountable for a Product and is never the Product's own. These
//! tests pin that framing and the two honesty rules that go with it --
//! every condition is attributed to what raised it, and the classification
//! fold names what forced it.

use pmc_application::attention_ranking::RankableAttentionItem;
use pmc_application::people_composition::RelationshipPurpose;
use pmc_application::product_composition::{
    compose_product_detail, current_conditions, effective_classification, flagged_carried_work,
    health_reason_subject, EvidenceFacts, PersonFacts, ProductDetailFacts, ProductFacts,
    StructureFacts,
};
use pmc_application::route_composition::{ComposedEntityKind, ImpactJudgment, OwnerModule};
use pmc_application::work_queue_composition::{WorkItemFacts, WorkItemKind};
use pmc_domain::attention::{
    AttentionFlag, AttentionMetadata, AttentionReason, AttentionTarget, Freshness,
};
use pmc_domain::classification::DataClassification;
use pmc_domain::identity::ActionId;
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::{EvidenceVerification, IntegrityDigest};

fn at(millis: i64) -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(millis)
}

fn product() -> ProductFacts {
    ProductFacts {
        id: "product-1".to_owned(),
        name: "Synthetic Product".to_owned(),
        classification: DataClassification::Internal,
        revision: 2,
        as_of: at(1_700_000_000_000),
    }
}

fn child(kind: ComposedEntityKind, id: &str, classification: DataClassification) -> StructureFacts {
    StructureFacts {
        kind,
        id: id.to_owned(),
        label: format!("Synthetic {id}"),
        classification,
        revision: 1,
        as_of: at(1_700_000_000_000),
        via: None,
    }
}

fn evidence(id: &str, verification: EvidenceVerification) -> EvidenceFacts {
    EvidenceFacts {
        id: id.to_owned(),
        role: None,
        verification,
        pinned: true,
        classification: DataClassification::Internal,
        classification_at_link: DataClassification::Internal,
        linked_at: at(6_000),
        revision: 3,
        as_of: at(1_699_000_000_000),
    }
}

fn verified() -> EvidenceVerification {
    EvidenceVerification::Verified {
        verified_at: at(7_000),
        integrity_digest: IntegrityDigest::parse(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
        )
        .unwrap(),
    }
}

fn overdue_action(id: &str, revision: u64, as_of: UtcTimestamp) -> WorkItemFacts {
    WorkItemFacts {
        kind: WorkItemKind::Action,
        id: id.to_owned(),
        label: format!("Action {id}"),
        state_label: "Open",
        lifecycle_legal_intents: vec!["start_action"],
        relevant_at: Some(at(1_000)),
        promised_at: None,
        classification: DataClassification::Internal,
        revision,
        as_of,
        freshness: Freshness::Fresh,
        degraded: false,
        attention: vec![RankableAttentionItem {
            flag: AttentionFlag {
                target: AttentionTarget::Action(ActionId::parse(id).unwrap()),
                reason: AttentionReason::ActionOverdue,
                metadata: AttentionMetadata {
                    classification: DataClassification::Internal,
                    freshness: Freshness::Fresh,
                    degraded: false,
                },
                explanation: "the action passed its due date",
                failed_verification_guidance: None,
            },
            relevant_at: Some(at(1_000)),
        }],
    }
}

fn person(
    id: &str,
    classification: DataClassification,
    carried: Vec<WorkItemFacts>,
) -> PersonFacts {
    PersonFacts {
        id: id.to_owned(),
        display_name: format!("Synthetic {id}"),
        purpose: RelationshipPurpose::Responsibility,
        classification,
        revision: 1,
        as_of: at(1_700_000_000_000),
        other_products_accountable_for: 0,
        carried,
    }
}

fn facts(
    structure: Vec<StructureFacts>,
    evidence: Vec<EvidenceFacts>,
    people: Vec<PersonFacts>,
) -> ProductDetailFacts {
    ProductDetailFacts {
        product: product(),
        structure,
        evidence,
        people,
    }
}

// ---------------------------------------------------------------------------
// The framing the amendment exists for.
// ---------------------------------------------------------------------------

#[test]
fn work_reaches_the_surface_only_under_the_person_who_carries_it() {
    // There is no field for "the Product's Actions". The only way an Action
    // appears is inside a person's `carried`, and it keeps its own identity
    // there.
    let composed = compose_product_detail(
        &facts(
            Vec::new(),
            Vec::new(),
            vec![person(
                "stakeholder-a",
                DataClassification::Internal,
                vec![overdue_action("action-1", 8, at(1_699_500_000_000))],
            )],
        ),
        9,
    )
    .unwrap();

    assert_eq!(composed.body.people.len(), 1);
    let carried = &composed.body.people[0].carried;
    assert_eq!(carried.len(), 1);
    assert_eq!(carried[0].kind, WorkItemKind::Action);
    assert_eq!(carried[0].id, "action-1");
    assert_eq!(carried[0].revision, 8);
    assert!(
        composed.body.product.attention.is_empty(),
        "the Product itself carries no flag"
    );
}

#[test]
fn impact_is_a_judgment_slot_and_is_reported_unassessed() {
    // No derivable source for `影響` exists. Reporting anything else here
    // would be inventing a judgment that belongs to a person.
    let composed = compose_product_detail(&facts(Vec::new(), Vec::new(), Vec::new()), 9).unwrap();
    assert_eq!(composed.body.impact, ImpactJudgment::Unassessed);
}

#[test]
fn an_initiative_keeps_the_project_it_was_reached_through() {
    let mut initiative = child(
        ComposedEntityKind::Initiative,
        "initiative-1",
        DataClassification::Internal,
    );
    initiative.via = Some("project:project-1".to_owned());

    let composed =
        compose_product_detail(&facts(vec![initiative], Vec::new(), Vec::new()), 9).unwrap();

    assert_eq!(
        composed.body.structure[0].via.as_deref(),
        Some("project:project-1")
    );
}

// ---------------------------------------------------------------------------
// `發生`: attributed to what raised it, never to the Product.
// ---------------------------------------------------------------------------

#[test]
fn an_unverified_evidence_link_is_a_condition_attributed_to_the_evidence() {
    let conditions = current_conditions(&facts(
        Vec::new(),
        vec![evidence("evidence-1", EvidenceVerification::Unverified)],
        Vec::new(),
    ))
    .unwrap();

    assert_eq!(conditions.len(), 1);
    assert!(conditions[0].value().contains("evidence-1"));
    assert!(conditions[0].value().contains("has not been verified"));
    let provenance = conditions[0].provenance();
    assert_eq!(provenance.owner(), OwnerModule::Evidence);
    assert_eq!(provenance.source_record_id(), "evidence-1");
    assert_eq!(provenance.source_revision(), 3);
    assert_eq!(provenance.as_of(), at(1_699_000_000_000));
}

#[test]
fn an_unpinned_observation_is_a_condition_that_names_the_missing_pin() {
    // The fifth verification state is reported as what it is -- read, but with no
    // pin to verify against -- attributed to the Evidence, never softened
    // into "verified" and never hardened into "mismatch".
    let mut unpinned = evidence(
        "evidence-1",
        EvidenceVerification::ObservedUnpinned {
            observed_at: at(5_000),
            integrity_digest: IntegrityDigest::parse("c".repeat(64)).unwrap(),
        },
    );
    unpinned.pinned = false;
    let conditions = current_conditions(&facts(Vec::new(), vec![unpinned], Vec::new())).unwrap();

    assert_eq!(conditions.len(), 1);
    assert!(conditions[0].value().contains("evidence-1"));
    assert!(conditions[0].value().contains("no pinned fingerprint"));
    assert!(!conditions[0].value().contains("verified,"));
    assert_eq!(conditions[0].provenance().owner(), OwnerModule::Evidence);
    assert_eq!(conditions[0].provenance().source_record_id(), "evidence-1");
}

#[test]
fn the_composed_entry_carries_pinned_as_its_own_fact() {
    let mut unpinned_but_verified = evidence("evidence-legacy", verified());
    unpinned_but_verified.pinned = false;
    let pinned = evidence("evidence-pinned", verified());
    let body = compose_product_detail(
        &facts(Vec::new(), vec![unpinned_but_verified, pinned], Vec::new()),
        7,
    )
    .unwrap()
    .body;

    let entry = |id: &str| {
        body.evidence
            .iter()
            .find(|entry| entry.entity.id == id)
            .unwrap_or_else(|| panic!("{id} must be composed"))
    };
    // A verified-looking reference without a pin is exactly the pre-v43
    // shape a surface must be able to flag; the fact travels independently.
    assert!(!entry("evidence-legacy").pinned);
    assert!(matches!(
        entry("evidence-legacy").verification,
        EvidenceVerification::Verified { .. }
    ));
    assert!(entry("evidence-pinned").pinned);
}

#[test]
fn verified_evidence_raises_no_condition() {
    let conditions = current_conditions(&facts(
        Vec::new(),
        vec![evidence("evidence-1", verified())],
        Vec::new(),
    ))
    .unwrap();
    assert!(conditions.is_empty());
}

#[test]
fn a_carried_flag_is_attributed_to_the_work_record_not_the_product() {
    // The overdue Action was read at revision 8, earlier than the Product.
    // The condition must carry the Action's provenance on every axis.
    let earlier = at(1_699_500_000_000);
    let conditions = current_conditions(&facts(
        Vec::new(),
        Vec::new(),
        vec![person(
            "stakeholder-a",
            DataClassification::Internal,
            vec![overdue_action("action-1", 8, earlier)],
        )],
    ))
    .unwrap();

    assert_eq!(conditions.len(), 1);
    let provenance = conditions[0].provenance();
    assert_eq!(provenance.owner(), OwnerModule::ActionManagement);
    assert_eq!(provenance.source_record_id(), "action-1");
    assert_eq!(provenance.source_revision(), 8);
    assert_eq!(provenance.as_of(), earlier);
    assert_ne!(provenance.source_revision(), product().revision);
}

#[test]
fn a_condition_never_claims_a_change_over_time() {
    // Nothing here reads history, so no condition may say "worsened",
    // "slipped" or "deteriorated". Pinned as a property over every condition
    // the two sources can produce.
    let conditions = current_conditions(&facts(
        Vec::new(),
        vec![
            evidence("evidence-1", EvidenceVerification::Unverified),
            evidence("evidence-2", EvidenceVerification::IntegrityMismatch),
        ],
        vec![person(
            "stakeholder-a",
            DataClassification::Internal,
            vec![overdue_action("action-1", 8, at(1_699_500_000_000))],
        )],
    ))
    .unwrap();

    for condition in &conditions {
        let text = condition.value().to_lowercase();
        for forbidden in ["worsen", "slipped", "deteriorat", "became", "since"] {
            assert!(
                !text.contains(forbidden),
                "{:?} claims a change over time",
                condition.value()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Classification folds upward and names what forced it.
// ---------------------------------------------------------------------------

#[test]
fn a_restricted_person_forces_the_fold_and_is_named() {
    let (classification, forced_by) = effective_classification(&facts(
        vec![child(
            ComposedEntityKind::Project,
            "project-1",
            DataClassification::Internal,
        )],
        Vec::new(),
        vec![person(
            "stakeholder-a",
            DataClassification::Restricted,
            Vec::new(),
        )],
    ));

    assert_eq!(classification, DataClassification::Restricted);
    let fold = forced_by.expect("a fold above the Product's own must be named");
    assert_eq!(fold.forced_by_kind, ComposedEntityKind::Stakeholder);
    assert_eq!(fold.forced_by_id, "stakeholder-a");
    assert_eq!(fold.classification, DataClassification::Restricted);
}

#[test]
fn an_unclassified_child_is_absorbing_and_is_named() {
    let (classification, forced_by) = effective_classification(&facts(
        vec![
            child(
                ComposedEntityKind::Project,
                "project-1",
                DataClassification::Restricted,
            ),
            child(
                ComposedEntityKind::Milestone,
                "milestone-1",
                DataClassification::Unclassified,
            ),
        ],
        Vec::new(),
        Vec::new(),
    ));

    assert_eq!(classification, DataClassification::Unclassified);
    let fold = forced_by.unwrap();
    assert_eq!(fold.forced_by_id, "milestone-1");
}

#[test]
fn the_link_time_classification_folds_too() {
    // A link recorded as Confidential against Internal Evidence and an
    // Internal Product is still something the surface exposes.
    let mut record = evidence("evidence-1", verified());
    record.classification_at_link = DataClassification::Confidential;

    let (classification, forced_by) =
        effective_classification(&facts(Vec::new(), vec![record], Vec::new()));

    assert_eq!(classification, DataClassification::Confidential);
    assert_eq!(forced_by.unwrap().forced_by_id, "evidence-1");
}

#[test]
fn nothing_stricter_than_the_product_means_no_fold_is_named() {
    let (classification, forced_by) = effective_classification(&facts(
        vec![child(
            ComposedEntityKind::Project,
            "project-1",
            DataClassification::Public,
        )],
        Vec::new(),
        Vec::new(),
    ));

    assert_eq!(classification, DataClassification::Internal);
    assert_eq!(forced_by, None);
}

#[test]
fn the_composed_product_is_presented_at_the_effective_classification() {
    let composed = compose_product_detail(
        &facts(
            Vec::new(),
            Vec::new(),
            vec![person(
                "stakeholder-a",
                DataClassification::Restricted,
                Vec::new(),
            )],
        ),
        9,
    )
    .unwrap();

    assert_eq!(
        composed.body.product.classification,
        DataClassification::Restricted
    );
    assert!(composed.body.classification_forced_by.is_some());
    assert_eq!(composed.body.product_label, "Synthetic Product");
    assert_eq!(composed.ledger_revision, 9);
}

// ---------------------------------------------------------------------------
// What a surface needs to word a condition, and to count carried work.
// ---------------------------------------------------------------------------

fn unverified() -> EvidenceVerification {
    EvidenceVerification::Unverified
}

#[test]
fn an_evidence_condition_is_read_back_as_its_verification_kind() {
    let body = compose_product_detail(
        &facts(
            Vec::new(),
            vec![evidence("evidence-1", unverified())],
            Vec::new(),
        ),
        9,
    )
    .unwrap()
    .body;

    let reason = &body.health_reasons[0];
    let subject = health_reason_subject(&body, reason).unwrap();
    assert_eq!(subject.reason_code, "unverified");
    assert_eq!(subject.subject_kind, "evidence");
    assert_eq!(subject.subject_label, "evidence-1");
}

#[test]
fn a_carried_condition_is_read_back_from_the_item_that_raised_it() {
    // Two people carry items whose flags share one explanation. Each reason
    // must be read back as its own record, not the first with that sentence.
    let first = overdue_action("action-1", 4, at(1_700_000_000_000));
    let mut second = overdue_action("action-2", 5, at(1_700_000_000_000));
    second.label = "The other action".to_owned();
    let body = compose_product_detail(
        &facts(
            Vec::new(),
            Vec::new(),
            vec![
                person("stakeholder-a", DataClassification::Internal, vec![first]),
                person("stakeholder-b", DataClassification::Internal, vec![second]),
            ],
        ),
        9,
    )
    .unwrap()
    .body;

    let labels: Vec<String> = body
        .health_reasons
        .iter()
        .map(|reason| {
            let subject = health_reason_subject(&body, reason).unwrap();
            assert_eq!(subject.reason_code, "action_overdue");
            assert_eq!(subject.subject_kind, "action");
            format!(
                "{} {}",
                reason.provenance().source_record_id(),
                subject.subject_label
            )
        })
        .collect();
    assert!(
        labels.contains(&"action-1 Action action-1".to_owned()),
        "{labels:?}"
    );
    assert!(
        labels.contains(&"action-2 The other action".to_owned()),
        "{labels:?}"
    );
}

#[test]
fn an_evidence_id_that_matches_a_work_id_is_not_confused_with_it() {
    // The source field, not the identifier alone, says which list to read.
    let body = compose_product_detail(
        &facts(
            Vec::new(),
            vec![evidence("action-1", unverified())],
            vec![person(
                "stakeholder-a",
                DataClassification::Internal,
                vec![overdue_action("action-1", 4, at(1_700_000_000_000))],
            )],
        ),
        9,
    )
    .unwrap()
    .body;

    let kinds: Vec<&str> = body
        .health_reasons
        .iter()
        .map(|reason| health_reason_subject(&body, reason).unwrap().subject_kind)
        .collect();
    assert!(kinds.contains(&"evidence"), "{kinds:?}");
    assert!(kinds.contains(&"action"), "{kinds:?}");
}

#[test]
fn carried_work_is_counted_once_per_record_and_folds_its_classification() {
    let mut restricted = overdue_action("action-2", 5, at(1_700_000_000_000));
    restricted.classification = DataClassification::Restricted;
    let mut quiet = overdue_action("action-3", 6, at(1_700_000_000_000));
    quiet.attention.clear();
    quiet.classification = DataClassification::Confidential;
    let shared = overdue_action("action-1", 4, at(1_700_000_000_000));
    let detail = facts(
        Vec::new(),
        Vec::new(),
        vec![
            person(
                "stakeholder-a",
                DataClassification::Internal,
                vec![shared.clone(), restricted],
            ),
            // The same item carried by a second person is still one item.
            person(
                "stakeholder-b",
                DataClassification::Internal,
                vec![shared, quiet],
            ),
        ],
    );

    let (count, classification) = flagged_carried_work(&detail);

    assert_eq!(count, 2);
    // The unflagged Confidential item is not counted, so it is not folded.
    assert_eq!(classification, Some(DataClassification::Restricted));
}

#[test]
fn no_flagged_work_is_zero_with_nothing_to_fold() {
    assert_eq!(
        flagged_carried_work(&facts(Vec::new(), Vec::new(), Vec::new())),
        (0, None)
    );
}
