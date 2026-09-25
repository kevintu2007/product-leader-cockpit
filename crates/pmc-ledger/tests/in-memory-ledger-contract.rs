use pmc_domain::audit::AuditEventIdSource;
use pmc_domain::classification::DataClassification;
use pmc_domain::delivery::{
    CreateInitiative, CreateProject, DefinedOutcome, OperationContext as DeliveryContext,
    RecordName,
};
use pmc_domain::error::ErrorCode;
use pmc_domain::execution::{
    ApprovalAuthorizationPort, ApproveAndExecuteRemoveRelationship, ExecutionIdSource,
    PrepareRemoveRelationship, RecoveryEvidence, RecoveryEvidencePort, RemovalPolicyPort,
};
use pmc_domain::identity::{
    AggregateVersion, AuditEventId, CorrelationId, IdempotencyId, InitiativeId, PortfolioId,
    PreparedIntentId, ProductId, ProjectId, RelationshipId, StakeholderId,
};
use pmc_domain::portfolio::{
    CreatePortfolio, CreateProduct, LongText, OperationContext, ShortText, UpdateProductDetails,
};
use pmc_domain::provenance::{Provenance, ProvenanceReference};
use pmc_domain::relationships::{
    CreateStakeholder, LinkInitiativeProject, LinkPortfolioProduct, LinkStakeholderRelationship,
    OperationContext as RelationshipContext, StakeholderKind, StakeholderRelationshipPurpose,
    StakeholderSubject,
};
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::DomainValueError;
use pmc_ledger::InMemoryProductLedger;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Clone, Copy)]
struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(1_908_072_600_000)
    }
}

#[derive(Clone)]
struct AuditIds {
    namespace: &'static str,
    next: u64,
    shared_sequence: Option<Arc<AtomicU64>>,
}
impl AuditEventIdSource for AuditIds {
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        let sequence = match &self.shared_sequence {
            Some(shared) => shared.fetch_add(1, Ordering::SeqCst) + 1,
            None => {
                self.next += 1;
                self.next
            }
        };
        let value = if self.namespace == "collision-delivery" && sequence == 1 {
            "collision-audit-1".to_owned()
        } else {
            format!("{}-audit-{sequence}", self.namespace)
        };
        AuditEventId::parse(value)
    }
}

fn portfolio_audits() -> AuditIds {
    AuditIds {
        namespace: "portfolio",
        next: 0,
        shared_sequence: None,
    }
}
fn delivery_audits() -> AuditIds {
    AuditIds {
        namespace: "delivery",
        next: 0,
        shared_sequence: None,
    }
}
fn relationship_audits() -> AuditIds {
    AuditIds {
        namespace: "relationship",
        next: 0,
        shared_sequence: None,
    }
}

fn ledger() -> InMemoryProductLedger<FixedClock, AuditIds> {
    InMemoryProductLedger::new(
        FixedClock,
        portfolio_audits(),
        delivery_audits(),
        relationship_audits(),
    )
}

fn context(id: &str) -> OperationContext {
    OperationContext {
        idempotency_id: IdempotencyId::parse(format!("ledger-idem-{id}")).expect("synthetic id"),
        correlation_id: CorrelationId::parse(format!("ledger-correlation-{id}"))
            .expect("synthetic id"),
    }
}
fn provenance() -> Provenance {
    Provenance::SyntheticFixture(
        ProvenanceReference::parse("synthetic-ledger-contract").expect("synthetic provenance"),
    )
}
fn short(value: &str) -> ShortText {
    ShortText::parse(value).expect("synthetic short text")
}
fn long(value: &str) -> LongText {
    LongText::parse(value).expect("synthetic long text")
}

fn delivery_context(id: &str) -> DeliveryContext {
    DeliveryContext {
        correlation_id: CorrelationId::parse(format!("delivery-correlation-{id}"))
            .expect("synthetic id"),
        idempotency_id: IdempotencyId::parse(format!("delivery-idem-{id}")).expect("synthetic id"),
    }
}
fn relationship_context(id: &str) -> RelationshipContext {
    RelationshipContext {
        correlation_id: CorrelationId::parse(format!("relationship-correlation-{id}"))
            .expect("synthetic id"),
        idempotency_id: IdempotencyId::parse(format!("relationship-idem-{id}"))
            .expect("synthetic id"),
    }
}
fn record_name(value: &str) -> RecordName {
    RecordName::parse(
        value,
        &CorrelationId::parse("name-correlation").expect("synthetic id"),
    )
    .expect("synthetic name")
}
fn outcome(value: &str) -> DefinedOutcome {
    DefinedOutcome::parse(
        value,
        &CorrelationId::parse("outcome-correlation").expect("synthetic id"),
    )
    .expect("synthetic outcome")
}

#[test]
fn typed_command_commits_and_inspect_reads_the_record() {
    let mut ledger = ledger();
    let id = PortfolioId::parse("portfolio-contract").expect("synthetic id");
    ledger
        .create_portfolio(CreatePortfolio {
            id: id.clone(),
            name: short("Synthetic portfolio"),
            details: long("contract fixture"),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
            context: context("create"),
        })
        .expect("create commits");
    assert_eq!(ledger.inspect_portfolio(&id).expect("record").id, id);
    assert_eq!(ledger.audit_events().len(), 1);
}

#[test]
fn outer_failure_rolls_back_and_retry_with_same_key_commits_once() {
    let mut ledger = ledger();
    ledger.inject_next_commit_failure();
    let id = ProductId::parse("product-contract").expect("synthetic id");
    let command = CreateProduct {
        id: id.clone(),
        name: short("Synthetic product"),
        details: long("contract fixture"),
        classification: Some(DataClassification::Public),
        provenance: provenance(),
        context: context("retry"),
    };
    let failure = ledger
        .create_product(command.clone())
        .expect_err("outer failure");
    assert!(failure.retryable());
    assert!(ledger.inspect_product(&id).is_none());
    assert!(ledger.audit_events().is_empty());
    ledger.create_product(command).expect("retry commits");
    assert_eq!(ledger.products().len(), 1);
    assert_eq!(ledger.audit_events().len(), 1);
}

#[test]
fn every_primary_aggregate_family_is_typed_and_queryable() {
    let mut ledger = ledger();
    let portfolio_id = PortfolioId::parse("portfolio-family").expect("synthetic id");
    let product_id = ProductId::parse("product-family").expect("synthetic id");
    let initiative_id = InitiativeId::parse("initiative-family").expect("synthetic id");
    let project_id = ProjectId::parse("project-family").expect("synthetic id");
    ledger
        .create_portfolio(CreatePortfolio {
            id: portfolio_id.clone(),
            name: short("Portfolio"),
            details: long("synthetic"),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
            context: context("family-portfolio"),
        })
        .expect("portfolio");
    ledger
        .create_product(CreateProduct {
            id: product_id.clone(),
            name: short("Product"),
            details: long("synthetic"),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
            context: context("family-product"),
        })
        .expect("product");
    ledger
        .create_initiative(CreateInitiative {
            context: delivery_context("family-initiative"),
            id: initiative_id.clone(),
            name: record_name("Initiative"),
            defined_outcome: outcome("Outcome"),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
        })
        .expect("initiative");
    ledger
        .create_project(CreateProject {
            context: delivery_context("family-project"),
            id: project_id.clone(),
            name: record_name("Project"),
            start_at: UtcTimestamp::from_unix_millis(100),
            end_at: UtcTimestamp::from_unix_millis(200),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
        })
        .expect("project");
    assert!(ledger.inspect_portfolio(&portfolio_id).is_some());
    assert!(ledger.inspect_product(&product_id).is_some());
    assert!(ledger.inspect_initiative(&initiative_id).is_some());
    assert!(ledger.inspect_project(&project_id).is_some());
    assert_eq!(ledger.portfolios().len(), 1);
    assert_eq!(ledger.products().len(), 1);
    assert_eq!(ledger.initiatives().len(), 1);
    assert_eq!(ledger.projects().len(), 1);
    assert_eq!(ledger.audit_events().len(), 4);
}

#[test]
fn named_links_and_stakeholder_purposes_use_fresh_staged_catalog() {
    let mut ledger = ledger();
    let portfolio_id = PortfolioId::parse("portfolio-links").expect("synthetic id");
    let product_id = ProductId::parse("product-links").expect("synthetic id");
    let initiative_id = InitiativeId::parse("initiative-links").expect("synthetic id");
    let project_id = ProjectId::parse("project-links").expect("synthetic id");
    let stakeholder_id = StakeholderId::parse("stakeholder-links").expect("synthetic id");
    ledger
        .create_portfolio(CreatePortfolio {
            id: portfolio_id.clone(),
            name: short("Portfolio"),
            details: long("synthetic"),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
            context: context("links-portfolio"),
        })
        .expect("portfolio");
    ledger
        .create_product(CreateProduct {
            id: product_id.clone(),
            name: short("Product"),
            details: long("synthetic"),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
            context: context("links-product"),
        })
        .expect("product");
    ledger
        .create_initiative(CreateInitiative {
            context: delivery_context("links-initiative"),
            id: initiative_id.clone(),
            name: record_name("Initiative"),
            defined_outcome: outcome("Outcome"),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
        })
        .expect("initiative");
    ledger
        .create_project(CreateProject {
            context: delivery_context("links-project"),
            id: project_id.clone(),
            name: record_name("Project"),
            start_at: UtcTimestamp::from_unix_millis(100),
            end_at: UtcTimestamp::from_unix_millis(200),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
        })
        .expect("project");
    ledger
        .create_stakeholder(CreateStakeholder {
            id: stakeholder_id.clone(),
            name: pmc_domain::relationships::StakeholderName::parse("Owner")
                .expect("synthetic name"),
            kind: StakeholderKind::Person,
            classification: Some(DataClassification::Public),
            provenance: provenance(),
            context: relationship_context("links-stakeholder"),
        })
        .expect("stakeholder");
    ledger
        .link_portfolio_product(LinkPortfolioProduct {
            id: RelationshipId::parse("relationship-portfolio-product").expect("synthetic id"),
            portfolio_id: portfolio_id.clone(),
            product_id: product_id.clone(),
            expected_portfolio_version: AggregateVersion::initial(),
            expected_product_version: AggregateVersion::initial(),
            context: relationship_context("portfolio-product"),
        })
        .expect("portfolio product link");
    ledger
        .link_initiative_project(LinkInitiativeProject {
            id: RelationshipId::parse("relationship-initiative-project").expect("synthetic id"),
            initiative_id: initiative_id.clone(),
            project_id: project_id.clone(),
            expected_initiative_version: AggregateVersion::initial(),
            expected_project_version: AggregateVersion::initial(),
            context: relationship_context("initiative-project"),
        })
        .expect("initiative project link");
    for (purpose, suffix) in [
        (
            StakeholderRelationshipPurpose::Responsibility,
            "responsibility",
        ),
        (StakeholderRelationshipPurpose::Dependency, "dependency"),
    ] {
        ledger
            .link_stakeholder_relationship(LinkStakeholderRelationship {
                id: RelationshipId::parse(format!("relationship-stakeholder-{suffix}"))
                    .expect("synthetic id"),
                stakeholder_id: stakeholder_id.clone(),
                subject: StakeholderSubject::Product(product_id.clone()),
                purpose,
                expected_stakeholder_version: AggregateVersion::initial(),
                expected_subject_version: AggregateVersion::initial(),
                context: relationship_context(suffix),
            })
            .expect("stakeholder relationship link");
    }
    ledger
        .update_product_details(UpdateProductDetails {
            id: product_id.clone(),
            expected_version: AggregateVersion::initial(),
            name: short("Product revised"),
            details: long("synthetic revised"),
            classification: Some(DataClassification::Public),
            context: context("links-product-update"),
        })
        .expect("product update");
    let stale_link = LinkPortfolioProduct {
        id: RelationshipId::parse("relationship-stale-product").expect("synthetic id"),
        portfolio_id: portfolio_id.clone(),
        product_id: product_id.clone(),
        expected_portfolio_version: AggregateVersion::initial(),
        expected_product_version: AggregateVersion::initial(),
        context: relationship_context("stale-product"),
    };
    assert!(ledger.link_portfolio_product(stale_link).is_err());
    ledger
        .link_portfolio_product(LinkPortfolioProduct {
            id: RelationshipId::parse("relationship-fresh-product").expect("synthetic id"),
            portfolio_id,
            product_id,
            expected_portfolio_version: AggregateVersion::initial(),
            expected_product_version: AggregateVersion::new(2).expect("version"),
            context: relationship_context("fresh-product"),
        })
        .expect("fresh endpoint version");
    assert_eq!(ledger.relationships().expect("relationships").len(), 4);
}

#[test]
fn domain_validation_error_does_not_consume_outer_failure_flag() {
    let mut ledger = ledger();
    let invalid = CreateProduct {
        id: ProductId::parse("product-invalid").expect("synthetic id"),
        name: short("Product"),
        details: long("synthetic"),
        classification: Some(DataClassification::Public),
        provenance: provenance(),
        context: context("seed"),
    };
    ledger.create_product(invalid.clone()).expect("seed");
    ledger.inject_next_commit_failure();
    ledger
        .create_product(CreateProduct {
            context: context("invalid"),
            ..invalid.clone()
        })
        .expect_err("duplicate domain validation");
    let valid = CreateProduct {
        id: ProductId::parse("product-valid-after-invalid").expect("synthetic id"),
        context: context("valid-after-invalid"),
        ..invalid
    };
    assert!(
        ledger.create_product(valid).is_err(),
        "outer failure remains armed"
    );
    assert!(ledger
        .inspect_product(&ProductId::parse("product-valid-after-invalid").expect("synthetic id"))
        .is_none());
}

#[derive(Clone, Copy)]
struct AllowRemoval;
impl RemovalPolicyPort for AllowRemoval {
    fn allow_relationship_removal(&self, _: &RelationshipId, _: DataClassification) -> bool {
        true
    }
}
#[derive(Clone, Copy)]
struct AllowApproval;
impl ApprovalAuthorizationPort for AllowApproval {
    fn authorize_relationship_removal(&self, actor: pmc_domain::audit::AuditActor) -> bool {
        actor == pmc_domain::audit::AuditActor::HeadOfProducts
    }
}
#[derive(Clone, Default)]
struct ExecutionIds(u64);
impl ExecutionIdSource for ExecutionIds {
    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<pmc_domain::identity::PreparedIntentId, DomainValueError> {
        self.0 += 1;
        pmc_domain::identity::PreparedIntentId::parse(format!("prepared-contract-{}", self.0))
    }
    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<pmc_domain::identity::ApprovalReceiptId, DomainValueError> {
        self.0 += 1;
        pmc_domain::identity::ApprovalReceiptId::parse(format!("receipt-contract-{}", self.0))
    }
}
#[derive(Clone)]
struct SyntheticRecovery(RecoveryEvidence);
impl RecoveryEvidencePort for SyntheticRecovery {
    fn recovery_evidence(&self, _: &RelationshipId) -> Option<RecoveryEvidence> {
        Some(self.0.clone())
    }
}

type H2bLedger =
    InMemoryProductLedger<FixedClock, AuditIds, ExecutionIds, AllowRemoval, AllowApproval>;

fn h2b_fixture() -> (
    H2bLedger,
    RelationshipId,
    pmc_domain::execution::PreparedIntent,
    SyntheticRecovery,
) {
    let portfolio_id = PortfolioId::parse("portfolio-h2b").expect("synthetic id");
    let product_id = ProductId::parse("product-h2b").expect("synthetic id");
    let relationship_id = RelationshipId::parse("relationship-h2b").expect("synthetic id");
    let mut ledger = InMemoryProductLedger::new_with_execution_authorities(
        FixedClock,
        portfolio_audits(),
        delivery_audits(),
        relationship_audits(),
        ExecutionIds::default(),
        AllowRemoval,
        AllowApproval,
    );
    ledger
        .create_portfolio(CreatePortfolio {
            id: portfolio_id.clone(),
            name: short("Portfolio"),
            details: long("synthetic"),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
            context: context("h2b-portfolio"),
        })
        .expect("portfolio");
    ledger
        .create_product(CreateProduct {
            id: product_id.clone(),
            name: short("Product"),
            details: long("synthetic"),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
            context: context("h2b-product"),
        })
        .expect("product");
    ledger
        .link_portfolio_product(LinkPortfolioProduct {
            id: relationship_id.clone(),
            portfolio_id,
            product_id,
            expected_portfolio_version: AggregateVersion::initial(),
            expected_product_version: AggregateVersion::initial(),
            context: relationship_context("h2b-link"),
        })
        .expect("link");
    let evidence = RecoveryEvidence::new(
        pmc_domain::identity::RecoveryEvidenceId::parse("evidence-h2b").expect("synthetic id"),
        "synthetic-recovery",
        UtcTimestamp::from_unix_millis(1),
        relationship_id.clone(),
        true,
    )
    .expect("synthetic evidence");
    let prepared = ledger
        .prepare_remove_relationship(
            PrepareRemoveRelationship {
                relationship_id: relationship_id.clone(),
                context: relationship_context("h2b-prepare"),
            },
            &SyntheticRecovery(evidence),
        )
        .expect("prepare");
    let recovery = SyntheticRecovery(prepared.preview().evidence().clone());
    (ledger, relationship_id, prepared, recovery)
}

#[test]
fn h2b_outer_failure_rolls_back_and_exact_replay_is_idempotent() {
    let (mut ledger, relationship_id, prepared, recovery) = h2b_fixture();
    let request = ApproveAndExecuteRemoveRelationship {
        prepared_id: prepared.id().clone(),
        actor: pmc_domain::audit::AuditActor::HeadOfProducts,
        confirmation: prepared.confirmation_challenge().to_owned(),
        acknowledged_payload_digest: prepared.payload_digest().clone(),
        context: relationship_context("h2b-execute"),
    };
    ledger.inject_next_commit_failure();
    assert!(ledger
        .approve_and_execute_remove_relationship(request.clone(), &recovery)
        .is_err());
    assert!(ledger
        .inspect_relationship(&relationship_id)
        .expect("inspect")
        .is_some());
    let outcome = ledger
        .approve_and_execute_remove_relationship(request.clone(), &recovery)
        .expect("retry");
    assert!(ledger
        .inspect_relationship(&relationship_id)
        .expect("inspect")
        .is_none());
    let replay = ledger
        .approve_and_execute_remove_relationship(request, &recovery)
        .expect("exact replay");
    assert_eq!(outcome, replay);
}

#[test]
fn h2b_audited_rejection_commits_once_rolls_back_outer_failure_and_allows_correction() {
    let (mut ledger, relationship_id, prepared, recovery) = h2b_fixture();
    let rejected_idempotency =
        IdempotencyId::parse("relationship-idem-h2b-denied").expect("synthetic id");
    let rejected = ApproveAndExecuteRemoveRelationship {
        prepared_id: prepared.id().clone(),
        actor: pmc_domain::audit::AuditActor::PolicyAuthorizedSystem,
        confirmation: prepared.confirmation_challenge().to_owned(),
        acknowledged_payload_digest: prepared.payload_digest().clone(),
        context: RelationshipContext {
            correlation_id: CorrelationId::parse("relationship-correlation-h2b-denied")
                .expect("synthetic id"),
            idempotency_id: rejected_idempotency.clone(),
        },
    };
    let audits_before = ledger.audit_events();

    ledger.inject_next_commit_failure();
    let outer_error = ledger
        .approve_and_execute_remove_relationship(rejected.clone(), &recovery)
        .expect_err("outer commit must fail");
    assert_eq!(outer_error.code(), ErrorCode::PlatformInternal);
    assert_eq!(ledger.audit_events(), audits_before);
    assert!(ledger
        .inspect_relationship(&relationship_id)
        .expect("inspect")
        .is_some());

    let denial = ledger
        .approve_and_execute_remove_relationship(rejected.clone(), &recovery)
        .expect_err("authorization denial");
    assert_eq!(denial.code(), ErrorCode::SecurityPolicyDenied);
    let audits_after_denial = ledger.audit_events();
    assert_eq!(audits_after_denial.len(), audits_before.len() + 1);
    assert_eq!(
        ledger
            .approve_and_execute_remove_relationship(rejected, &recovery)
            .expect_err("exact denial replay"),
        denial
    );
    assert_eq!(ledger.audit_events(), audits_after_denial);

    let same_key_changed_actor = ApproveAndExecuteRemoveRelationship {
        prepared_id: prepared.id().clone(),
        actor: pmc_domain::audit::AuditActor::HeadOfProducts,
        confirmation: prepared.confirmation_challenge().to_owned(),
        acknowledged_payload_digest: prepared.payload_digest().clone(),
        context: RelationshipContext {
            correlation_id: CorrelationId::parse("relationship-correlation-h2b-denied-retry")
                .expect("synthetic id"),
            idempotency_id: rejected_idempotency.clone(),
        },
    };
    assert_eq!(
        ledger
            .approve_and_execute_remove_relationship(same_key_changed_actor, &recovery)
            .expect_err("changed actor conflicts with the terminal command")
            .code(),
        ErrorCode::DomainIdempotencyConflict
    );
    assert_eq!(ledger.audit_events(), audits_after_denial);

    let same_key_changed_confirmation = ApproveAndExecuteRemoveRelationship {
        prepared_id: prepared.id().clone(),
        actor: pmc_domain::audit::AuditActor::PolicyAuthorizedSystem,
        confirmation: "REMOVE DIFFERENT SYNTHETIC RELATIONSHIP".to_owned(),
        acknowledged_payload_digest: prepared.payload_digest().clone(),
        context: RelationshipContext {
            correlation_id: CorrelationId::parse(
                "relationship-correlation-h2b-confirmation-changed",
            )
            .expect("synthetic id"),
            idempotency_id: rejected_idempotency.clone(),
        },
    };
    assert_eq!(
        ledger
            .approve_and_execute_remove_relationship(same_key_changed_confirmation, &recovery)
            .expect_err("changed confirmation conflicts with the terminal command")
            .code(),
        ErrorCode::DomainIdempotencyConflict
    );
    assert_eq!(ledger.audit_events(), audits_after_denial);

    let same_key_changed_prepared = ApproveAndExecuteRemoveRelationship {
        prepared_id: PreparedIntentId::parse("prepared-intent-changed").expect("synthetic id"),
        actor: pmc_domain::audit::AuditActor::PolicyAuthorizedSystem,
        confirmation: prepared.confirmation_challenge().to_owned(),
        acknowledged_payload_digest: prepared.payload_digest().clone(),
        context: RelationshipContext {
            correlation_id: CorrelationId::parse("relationship-correlation-h2b-prepared-changed")
                .expect("synthetic id"),
            idempotency_id: rejected_idempotency.clone(),
        },
    };
    assert_eq!(
        ledger
            .approve_and_execute_remove_relationship(same_key_changed_prepared, &recovery)
            .expect_err("changed Prepared Intent conflicts with the terminal command")
            .code(),
        ErrorCode::DomainIdempotencyConflict
    );
    assert_eq!(ledger.audit_events(), audits_after_denial);

    let changed_digest =
        pmc_domain::execution::PayloadDigest::parse("0".repeat(64)).expect("synthetic digest");
    let same_key_changed_digest = ApproveAndExecuteRemoveRelationship {
        prepared_id: prepared.id().clone(),
        actor: pmc_domain::audit::AuditActor::PolicyAuthorizedSystem,
        confirmation: prepared.confirmation_challenge().to_owned(),
        acknowledged_payload_digest: changed_digest,
        context: RelationshipContext {
            correlation_id: CorrelationId::parse("relationship-correlation-h2b-digest-changed")
                .expect("synthetic id"),
            idempotency_id: rejected_idempotency.clone(),
        },
    };
    assert_eq!(
        ledger
            .approve_and_execute_remove_relationship(same_key_changed_digest, &recovery)
            .expect_err("changed digest conflicts with the terminal command")
            .code(),
        ErrorCode::DomainIdempotencyConflict
    );
    assert_eq!(ledger.audit_events(), audits_after_denial);

    let cross_family = ledger.create_portfolio(CreatePortfolio {
        id: PortfolioId::parse("portfolio-after-h2b-denial").expect("synthetic id"),
        name: short("Portfolio"),
        details: long("synthetic"),
        classification: Some(DataClassification::Public),
        provenance: provenance(),
        context: OperationContext {
            idempotency_id: rejected_idempotency,
            correlation_id: CorrelationId::parse("portfolio-correlation-after-h2b-denial")
                .expect("synthetic id"),
        },
    });
    assert_eq!(
        cross_family
            .expect_err("global idempotency namespace")
            .code(),
        ErrorCode::DomainIdempotencyConflict
    );

    let corrected = ApproveAndExecuteRemoveRelationship {
        prepared_id: prepared.id().clone(),
        actor: pmc_domain::audit::AuditActor::HeadOfProducts,
        confirmation: prepared.confirmation_challenge().to_owned(),
        acknowledged_payload_digest: prepared.payload_digest().clone(),
        context: relationship_context("h2b-corrected"),
    };
    ledger
        .approve_and_execute_remove_relationship(corrected, &recovery)
        .expect("corrected approval");
    assert!(ledger
        .inspect_relationship(&relationship_id)
        .expect("inspect")
        .is_none());
}

#[test]
fn global_idempotency_blocks_cross_family_reuse_and_preserves_replay_flag() {
    let mut ledger = ledger();
    let shared = "ledger-idem-cross-family-shared";
    let portfolio_id = PortfolioId::parse("portfolio-cross-family").expect("synthetic id");
    let portfolio_command = CreatePortfolio {
        id: portfolio_id.clone(),
        name: short("Portfolio"),
        details: long("synthetic"),
        classification: Some(DataClassification::Public),
        provenance: provenance(),
        context: pmc_domain::portfolio::OperationContext {
            idempotency_id: IdempotencyId::parse(shared).expect("synthetic id"),
            correlation_id: CorrelationId::parse("cross-family-portfolio-correlation")
                .expect("synthetic id"),
        },
    };
    let replay = ledger
        .create_portfolio(portfolio_command.clone())
        .expect("first command");
    let cross_family = CreateInitiative {
        context: DeliveryContext {
            correlation_id: CorrelationId::parse("cross-family-delivery-correlation")
                .expect("synthetic id"),
            idempotency_id: IdempotencyId::parse(shared).expect("synthetic id"),
        },
        id: InitiativeId::parse("initiative-cross-family").expect("synthetic id"),
        name: record_name("Initiative"),
        defined_outcome: outcome("Outcome"),
        classification: Some(DataClassification::Public),
        provenance: provenance(),
    };
    assert!(ledger.create_initiative(cross_family).is_err());
    ledger.inject_next_commit_failure();
    let replay_again = ledger
        .create_portfolio(portfolio_command)
        .expect("same-family exact replay");
    assert_eq!(replay.record, replay_again.record);
    let new_product = CreateProduct {
        id: ProductId::parse("product-after-replay").expect("synthetic id"),
        name: short("Product"),
        details: long("synthetic"),
        classification: Some(DataClassification::Public),
        provenance: provenance(),
        context: context("new-after-replay"),
    };
    assert!(ledger.create_product(new_product).is_err());
    assert!(ledger
        .inspect_product(&ProductId::parse("product-after-replay").expect("synthetic id"))
        .is_none());
}

#[test]
fn audit_stream_preserves_commit_order_and_rejects_staged_collisions() {
    let mut ledger = ledger();
    ledger
        .create_portfolio(CreatePortfolio {
            id: PortfolioId::parse("portfolio-audit-order").expect("synthetic id"),
            name: short("Portfolio"),
            details: long("synthetic"),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
            context: context("audit-order-portfolio"),
        })
        .expect("portfolio");
    ledger
        .create_product(CreateProduct {
            id: ProductId::parse("product-audit-order").expect("synthetic id"),
            name: short("Product"),
            details: long("synthetic"),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
            context: context("audit-order-product"),
        })
        .expect("product");
    let audits = ledger.audit_events();
    assert_eq!(audits.len(), 2);
    assert!(audits[0].id().as_str().starts_with("portfolio-"));
    assert!(audits[1].id().as_str().starts_with("portfolio-"));

    let mut collision_ledger = InMemoryProductLedger::new(
        FixedClock,
        AuditIds {
            namespace: "collision",
            next: 0,
            shared_sequence: None,
        },
        AuditIds {
            namespace: "collision-delivery",
            next: 0,
            shared_sequence: Some(Arc::new(AtomicU64::new(0))),
        },
        AuditIds {
            namespace: "relationship-collision",
            next: 0,
            shared_sequence: None,
        },
    );
    collision_ledger
        .create_portfolio(CreatePortfolio {
            id: PortfolioId::parse("portfolio-collision").expect("synthetic id"),
            name: short("Portfolio"),
            details: long("synthetic"),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
            context: context("collision-portfolio"),
        })
        .expect("portfolio");
    let before = collision_ledger.audit_events();
    let error = collision_ledger
        .create_initiative(CreateInitiative {
            context: delivery_context("collision-initiative"),
            id: InitiativeId::parse("initiative-collision").expect("synthetic id"),
            name: record_name("Initiative"),
            defined_outcome: outcome("Outcome"),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
        })
        .expect_err("staged audit collision");
    assert_eq!(error.code(), ErrorCode::PlatformInternal);
    assert!(error.retryable());
    assert!(collision_ledger
        .inspect_initiative(&InitiativeId::parse("initiative-collision").expect("synthetic id"))
        .is_none());
    assert_eq!(collision_ledger.audit_events(), before);
    collision_ledger
        .create_initiative(CreateInitiative {
            context: delivery_context("collision-initiative-retry"),
            id: InitiativeId::parse("initiative-collision").expect("synthetic id"),
            name: record_name("Initiative retry"),
            defined_outcome: outcome("Outcome retry"),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
        })
        .expect("source advances outside staged state and retry succeeds");
}
