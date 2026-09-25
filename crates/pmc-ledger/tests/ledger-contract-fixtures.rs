use pmc_domain::audit::AuditActor;
use pmc_domain::classification::DataClassification;
use pmc_domain::delivery::UpdateProject;
use pmc_domain::execution::{
    ApproveAndExecuteRemoveRelationship, PrepareRemoveRelationship, UnavailableRecoveryEvidence,
};
use pmc_domain::identity::{AggregateVersion, CorrelationId, IdempotencyId, ProductId};
use pmc_domain::portfolio::{
    CreateProduct, LongText, OperationContext, ShortText, UpdateProductDetails,
};
use pmc_domain::provenance::{Provenance, ProvenanceReference};
use pmc_domain::relationships::{
    LinkPortfolioProduct, OperationContext as RelationshipContext, ValidateProjectMilestone,
};
mod support;
use support::SyntheticPmc003Fixture;

fn correlation(value: &str) -> CorrelationId {
    CorrelationId::parse(format!("synthetic-test-correlation-{value}"))
        .unwrap_or_else(|_| unreachable!())
}
fn portfolio_context(value: &str) -> OperationContext {
    OperationContext {
        correlation_id: correlation(value),
        idempotency_id: IdempotencyId::parse(format!("synthetic-test-idem-{value}"))
            .unwrap_or_else(|_| unreachable!()),
    }
}
fn relationship_context(value: &str) -> RelationshipContext {
    RelationshipContext {
        correlation_id: correlation(value),
        idempotency_id: IdempotencyId::parse(format!("synthetic-test-rel-idem-{value}"))
            .unwrap_or_else(|_| unreachable!()),
    }
}
fn public_provenance() -> Provenance {
    Provenance::SyntheticFixture(
        ProvenanceReference::parse("synthetic-pmc003-test").unwrap_or_else(|_| unreachable!()),
    )
}

#[test]
fn normal_ledger_surface_does_not_export_synthetic_fixture_authority() {
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
        .unwrap_or_else(|_| unreachable!());
    assert!(!source.contains("pub mod fixtures"));
    assert!(!source.contains("SyntheticPmc003Fixture"));
    assert!(!source.contains("SyntheticRemovalPolicy"));
    assert!(!source.contains("SyntheticApprovalAuthorization"));
    assert!(!source.contains("SyntheticRecoveryEvidence"));
}
fn short(value: &str) -> ShortText {
    ShortText::parse(value).unwrap_or_else(|_| unreachable!())
}
fn long(value: &str) -> LongText {
    LongText::parse(value).unwrap_or_else(|_| unreachable!())
}

#[test]
fn fixture_has_exact_typed_aggregate_and_relationship_counts() {
    let fixture = SyntheticPmc003Fixture::build().expect("synthetic fixture builds");
    let handles = fixture.handles();
    let ledger = fixture.ledger();
    assert_eq!(ledger.portfolios().len(), 1);
    assert_eq!(ledger.products().len(), 1);
    assert_eq!(ledger.roadmaps().len(), 1);
    assert_eq!(ledger.kpi_definitions().len(), 1);
    assert_eq!(ledger.kpi_observations().len(), 1);
    assert_eq!(ledger.initiatives().len(), 1);
    assert_eq!(ledger.projects().len(), 1);
    assert_eq!(ledger.milestones().len(), 1);
    assert_eq!(ledger.stakeholders().len(), 1);
    assert_eq!(ledger.relationships().expect("relationships").len(), 8);
    assert_eq!(ledger.audit_events().len(), 17);
    assert_eq!(
        ledger
            .inspect_project(&handles.project)
            .expect("project")
            .classification(),
        DataClassification::Internal
    );
    assert_eq!(
        ledger
            .inspect_milestone(&handles.milestone)
            .expect("milestone")
            .classification(),
        DataClassification::Internal
    );
    assert_eq!(
        ledger
            .inspect_kpi_observation(&handles.observation)
            .expect("observation")
            .classification,
        DataClassification::Confidential
    );
    let kpi = ledger.inspect_kpi_definition(&handles.kpi).expect("kpi");
    assert_eq!(kpi.name.as_str(), "Synthetic KPI Alpha");
    assert_eq!(kpi.target.as_str(), "Synthetic target");
    assert_eq!(kpi.cadence.as_str(), "Weekly");
    assert_eq!(kpi.source.as_str(), "Synthetic source");
    let observation = ledger
        .inspect_kpi_observation(&handles.observation)
        .expect("observation");
    assert_eq!(observation.value.as_str(), "42");
    assert_eq!(observation.source.as_str(), "Synthetic observation source");
    assert_eq!(observation.kpi_id, handles.kpi);
}

#[test]
fn project_milestone_is_a_validation_query_and_inherits_project_classification() {
    let mut fixture = SyntheticPmc003Fixture::build().expect("synthetic fixture builds");
    let handles = fixture.handles().clone();
    let validation = fixture
        .ledger_mut()
        .validate_project_milestone(ValidateProjectMilestone {
            project_id: handles.project,
            milestone_id: handles.milestone,
            expected_project_version: AggregateVersion::initial(),
            expected_milestone_version: AggregateVersion::initial(),
            context: relationship_context("project-milestone-validation"),
        })
        .expect("validation query");
    assert_eq!(validation.classification(), DataClassification::Internal);
    assert_eq!(
        fixture
            .ledger()
            .relationships()
            .expect("relationships")
            .len(),
        8
    );
}

#[test]
fn project_classification_raise_propagates_to_milestone_version_and_classification() {
    let mut fixture = SyntheticPmc003Fixture::build().expect("synthetic fixture builds");
    let handles = fixture.handles().clone();
    fixture
        .ledger_mut()
        .update_project(UpdateProject {
            context: pmc_domain::delivery::OperationContext {
                correlation_id: correlation("raise-project"),
                idempotency_id: IdempotencyId::parse("synthetic-test-delivery-idem-raise-project")
                    .unwrap_or_else(|_| unreachable!()),
            },
            id: handles.project.clone(),
            expected_version: AggregateVersion::initial(),
            name: pmc_domain::delivery::RecordName::parse(
                "Synthetic Project Alpha Raised",
                &correlation("raise-name"),
            )
            .unwrap_or_else(|_| unreachable!()),
            start_at: pmc_domain::time::UtcTimestamp::from_unix_millis(1_912_464_000_000),
            end_at: pmc_domain::time::UtcTimestamp::from_unix_millis(1_912_550_400_000),
            classification: Some(DataClassification::Restricted),
            provenance: public_provenance(),
        })
        .expect("project classification raise");
    let project = fixture
        .ledger()
        .inspect_project(&handles.project)
        .expect("project");
    let milestone = fixture
        .ledger()
        .inspect_milestone(&handles.milestone)
        .expect("milestone");
    assert_eq!(project.version().get(), 2);
    assert_eq!(milestone.version().get(), 2);
    assert_eq!(milestone.classification(), DataClassification::Restricted);
}

#[test]
fn fixture_ids_and_audits_are_stable_across_builds() {
    let a = SyntheticPmc003Fixture::build().expect("fixture");
    let b = SyntheticPmc003Fixture::build().expect("fixture");
    assert_eq!(a.handles(), b.handles());
    let a_audits = a.ledger().audit_events();
    let b_audits = b.ledger().audit_events();
    let a_ids: Vec<_> = a_audits
        .iter()
        .map(|event| event.id().to_string())
        .collect();
    let b_ids: Vec<_> = b_audits
        .iter()
        .map(|event| event.id().to_string())
        .collect();
    assert_eq!(a_ids, b_ids);
}

#[test]
fn default_recovery_port_fails_closed_and_fixture_port_prepares_h2b() {
    let mut fixture = SyntheticPmc003Fixture::build().expect("fixture");
    let relationship_id = fixture.handles().portfolio_product.clone();
    let recovery = fixture.recovery_for(&relationship_id);
    let denied = fixture.ledger_mut().prepare_remove_relationship(
        PrepareRemoveRelationship {
            relationship_id: relationship_id.clone(),
            context: relationship_context("default-denied"),
        },
        &UnavailableRecoveryEvidence,
    );
    assert!(denied.is_err());
    let prepared = fixture
        .ledger_mut()
        .prepare_remove_relationship(
            PrepareRemoveRelationship {
                relationship_id: relationship_id.clone(),
                context: relationship_context("fixture-prepare"),
            },
            &recovery,
        )
        .expect("synthetic evidence prepares");
    let request = ApproveAndExecuteRemoveRelationship {
        prepared_id: prepared.id().clone(),
        actor: AuditActor::HeadOfProducts,
        confirmation: prepared.confirmation_challenge().to_owned(),
        acknowledged_payload_digest: prepared.payload_digest().clone(),
        context: relationship_context("fixture-execute"),
    };
    fixture
        .ledger_mut()
        .cancel_remove_relationship(
            prepared.id(),
            AuditActor::HeadOfProducts,
            relationship_context("fixture-cancel"),
        )
        .expect("pre-approval cancellation");
    assert_eq!(
        fixture
            .ledger()
            .relationships()
            .expect("relationships")
            .len(),
        8
    );
    let _ = request;
}

#[test]
fn allowed_fixture_h2b_executes_replays_exactly_and_rejects_late_cancel() {
    let mut fixture = SyntheticPmc003Fixture::build().expect("fixture");
    let relationship_id = fixture.handles().portfolio_product.clone();
    let recovery = fixture.recovery_for(&relationship_id);
    let prepared = fixture
        .ledger_mut()
        .prepare_remove_relationship(
            PrepareRemoveRelationship {
                relationship_id: relationship_id.clone(),
                context: relationship_context("execute-prepare"),
            },
            &recovery,
        )
        .expect("prepare");
    let request = ApproveAndExecuteRemoveRelationship {
        prepared_id: prepared.id().clone(),
        actor: AuditActor::HeadOfProducts,
        confirmation: prepared.confirmation_challenge().to_owned(),
        acknowledged_payload_digest: prepared.payload_digest().clone(),
        context: relationship_context("execute-submit"),
    };
    let first = fixture
        .ledger_mut()
        .approve_and_execute_remove_relationship(request.clone(), &recovery)
        .expect("execute");
    assert_eq!(first.relationship_id, relationship_id);
    assert_eq!(first.audit_event_ids.len(), 1);
    assert_eq!(
        fixture
            .ledger()
            .relationships()
            .expect("relationships")
            .len(),
        7
    );
    assert_eq!(fixture.ledger().audit_events().len(), 18);
    let replay = fixture
        .ledger_mut()
        .approve_and_execute_remove_relationship(request, &recovery)
        .expect("exact replay");
    assert_eq!(replay, first);
    assert_eq!(
        fixture
            .ledger()
            .relationships()
            .expect("relationships")
            .len(),
        7
    );
    assert_eq!(fixture.ledger().audit_events().len(), 18);
    assert!(fixture
        .ledger_mut()
        .cancel_remove_relationship(
            prepared.id(),
            AuditActor::HeadOfProducts,
            relationship_context("late-cancel"),
        )
        .is_err());
    assert_eq!(
        fixture
            .ledger()
            .relationships()
            .expect("relationships")
            .len(),
        7
    );
}

#[test]
fn exact_create_and_link_retries_are_idempotent_and_add_no_audit() {
    let mut fixture = SyntheticPmc003Fixture::build().expect("fixture");
    let handles = fixture.handles().clone();
    let product = CreateProduct {
        id: ProductId::parse("synthetic-product-retry").unwrap_or_else(|_| unreachable!()),
        name: short("Synthetic Product Retry"),
        details: long("Synthetic exact retry probe"),
        classification: Some(DataClassification::Public),
        provenance: public_provenance(),
        context: portfolio_context("retry-create"),
    };
    fixture
        .ledger_mut()
        .create_product(product.clone())
        .expect("first create");
    let audits_after_create = fixture.ledger().audit_events().len();
    fixture
        .ledger_mut()
        .create_product(product)
        .expect("exact create retry");
    assert_eq!(fixture.ledger().products().len(), 2);
    assert_eq!(fixture.ledger().audit_events().len(), audits_after_create);
    let link = LinkPortfolioProduct {
        id: pmc_domain::identity::RelationshipId::parse("synthetic-rel-retry")
            .unwrap_or_else(|_| unreachable!()),
        portfolio_id: handles.portfolio,
        product_id: ProductId::parse("synthetic-product-retry").unwrap_or_else(|_| unreachable!()),
        expected_portfolio_version: AggregateVersion::initial(),
        expected_product_version: AggregateVersion::initial(),
        context: relationship_context("retry-link"),
    };
    fixture
        .ledger_mut()
        .link_portfolio_product(link.clone())
        .expect("first link");
    let audits_after_link = fixture.ledger().audit_events().len();
    fixture
        .ledger_mut()
        .link_portfolio_product(link)
        .expect("exact link retry");
    assert_eq!(
        fixture
            .ledger()
            .relationships()
            .expect("relationships")
            .len(),
        9
    );
    assert_eq!(fixture.ledger().audit_events().len(), audits_after_link);
}

#[test]
fn stale_link_and_outer_failure_leave_observable_bundle_unchanged() {
    let mut fixture = SyntheticPmc003Fixture::build().expect("fixture");
    let handles = fixture.handles().clone();
    fixture.ledger_mut().inject_next_commit_failure();
    let result = fixture.ledger_mut().create_product(CreateProduct {
        id: ProductId::parse("synthetic-product-rollback-probe").unwrap_or_else(|_| unreachable!()),
        name: short("Synthetic rollback probe"),
        details: long("Synthetic rollback only"),
        classification: Some(DataClassification::Public),
        provenance: public_provenance(),
        context: portfolio_context("rollback"),
    });
    assert!(result.is_err());
    assert_eq!(fixture.ledger().products().len(), 1);
    assert_eq!(
        fixture
            .ledger()
            .relationships()
            .expect("relationships")
            .len(),
        8
    );
    fixture
        .ledger_mut()
        .update_product_details(UpdateProductDetails {
            id: handles.product.clone(),
            expected_version: AggregateVersion::initial(),
            name: short("Synthetic Product Alpha Raised"),
            details: long("Synthetic stale-link endpoint version probe"),
            classification: Some(DataClassification::Internal),
            context: portfolio_context("raise-product-version"),
        })
        .expect("raise product version");
    let before_stale_audits = fixture.ledger().audit_events().len();
    let stale = fixture
        .ledger_mut()
        .link_portfolio_product(LinkPortfolioProduct {
            id: pmc_domain::identity::RelationshipId::parse("synthetic-rel-stale-probe")
                .unwrap_or_else(|_| unreachable!()),
            portfolio_id: handles.portfolio,
            product_id: handles.product.clone(),
            expected_portfolio_version: AggregateVersion::initial(),
            expected_product_version: AggregateVersion::initial(),
            context: relationship_context("stale-link"),
        });
    assert!(stale.is_err());
    assert_eq!(
        fixture
            .ledger()
            .relationships()
            .expect("relationships")
            .len(),
        8
    );
    assert_eq!(fixture.ledger().audit_events().len(), before_stale_audits);
    let stale_update = fixture
        .ledger_mut()
        .update_product_details(UpdateProductDetails {
            id: handles.product.clone(),
            expected_version: AggregateVersion::initial(),
            name: short("Synthetic Stale Update"),
            details: long("Synthetic stale aggregate probe"),
            classification: Some(DataClassification::Restricted),
            context: portfolio_context("stale-product-update"),
        });
    assert!(stale_update.is_err());
    let product = fixture
        .ledger()
        .inspect_product(&handles.product)
        .expect("product");
    assert_eq!(product.version.get(), 2);
    assert_eq!(product.name.as_str(), "Synthetic Product Alpha Raised");
    assert_eq!(fixture.ledger().audit_events().len(), before_stale_audits);
}
