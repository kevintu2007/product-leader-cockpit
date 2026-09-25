use pmc_domain::audit::{
    AuditActor, AuditEventIdSource, AuditExecutionOutcome, AuditModule, AuditTarget,
};
use pmc_domain::classification::DataClassification;
use pmc_domain::error::ErrorCode;
use pmc_domain::identity::{
    AggregateVersion, AuditEventId, CorrelationId, IdempotencyId, KpiId, KpiObservationId,
    PortfolioId, ProductId, RoadmapId,
};
use pmc_domain::portfolio::*;
use pmc_domain::provenance::{Provenance, ProvenanceReference};
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::DomainValueError;

#[derive(Clone, Copy)]
struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(1_908_072_600_000)
    }
}

struct SequentialAuditIds(u64);

impl AuditEventIdSource for SequentialAuditIds {
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        self.0 += 1;
        AuditEventId::parse(format!("audit-{}", self.0))
    }
}

fn service() -> InMemoryPortfolioService<FixedClock, SequentialAuditIds> {
    InMemoryPortfolioService::new(FixedClock, SequentialAuditIds(0))
}

fn short(value: &str) -> ShortText {
    ShortText::parse(value).unwrap_or_else(|error| panic!("synthetic short text: {error}"))
}

fn long(value: &str) -> LongText {
    LongText::parse(value).unwrap_or_else(|error| panic!("synthetic long text: {error}"))
}

fn context(id: &str) -> OperationContext {
    OperationContext {
        idempotency_id: IdempotencyId::parse(id)
            .unwrap_or_else(|error| panic!("synthetic idempotency id: {error}")),
        correlation_id: CorrelationId::parse(format!("correlation-{id}"))
            .unwrap_or_else(|error| panic!("synthetic correlation id: {error}")),
    }
}

fn provenance() -> Provenance {
    Provenance::SyntheticFixture(
        ProvenanceReference::parse("synthetic-portfolio-v1")
            .unwrap_or_else(|error| panic!("synthetic provenance: {error}")),
    )
}

fn portfolio_id() -> PortfolioId {
    PortfolioId::parse("portfolio-alpha")
        .unwrap_or_else(|error| panic!("synthetic portfolio id: {error}"))
}

fn product_id() -> ProductId {
    ProductId::parse("product-orbit")
        .unwrap_or_else(|error| panic!("synthetic product id: {error}"))
}

fn roadmap_id() -> RoadmapId {
    RoadmapId::parse("roadmap-orbit")
        .unwrap_or_else(|error| panic!("synthetic roadmap id: {error}"))
}

fn kpi_id() -> KpiId {
    KpiId::parse("kpi-adoption").unwrap_or_else(|error| panic!("synthetic KPI id: {error}"))
}

fn observation_id() -> KpiObservationId {
    KpiObservationId::parse("observation-june")
        .unwrap_or_else(|error| panic!("synthetic observation id: {error}"))
}

#[test]
fn typed_portfolio_product_and_roadmap_records_create_update_and_inspect() {
    let mut service = service();

    let portfolio = service
        .create_portfolio(CreatePortfolio {
            id: portfolio_id(),
            name: short("Synthetic Product Portfolio"),
            details: long("Public-safe portfolio used only by deterministic tests."),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
            context: context("create-portfolio"),
        })
        .unwrap_or_else(|error| panic!("create portfolio: {error}"));
    assert_eq!(portfolio.record.classification, DataClassification::Public);
    assert_eq!(portfolio.record.version, AggregateVersion::initial());

    let portfolio = service
        .update_portfolio_details(UpdatePortfolioDetails {
            id: portfolio_id(),
            expected_version: AggregateVersion::initial(),
            name: short("Synthetic Portfolio 2030"),
            details: long("A revised, synthetic-only operating scope."),
            classification: Some(DataClassification::Public),
            context: context("update-portfolio"),
        })
        .unwrap_or_else(|error| panic!("update portfolio: {error}"));
    assert_eq!(portfolio.record.version.get(), 2);
    assert_eq!(portfolio.record.classification, DataClassification::Public);
    assert_eq!(
        service.inspect_portfolio(&portfolio_id()),
        Some(portfolio.record)
    );

    service
        .create_product(CreateProduct {
            id: product_id(),
            name: short("Orbit Assistant"),
            details: long("Synthetic product outcome."),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
            context: context("create-product"),
        })
        .unwrap_or_else(|error| panic!("create product: {error}"));
    let product = service
        .update_product_details(UpdateProductDetails {
            id: product_id(),
            expected_version: AggregateVersion::initial(),
            name: short("Orbit Assistant Alpha"),
            details: long("Synthetic product outcome revised."),
            classification: None,
            context: context("update-product"),
        })
        .unwrap_or_else(|error| panic!("update product: {error}"));
    assert_eq!(product.record.version.get(), 2);
    assert_eq!(product.record.classification, DataClassification::Public);
    assert_eq!(service.inspect_product(&product_id()), Some(product.record));

    service
        .create_roadmap(CreateRoadmap {
            id: roadmap_id(),
            name: short("Outcome Roadmap"),
            details: long("Synthetic outcome sequence."),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
            context: context("create-roadmap"),
        })
        .unwrap_or_else(|error| panic!("create roadmap: {error}"));
    let roadmap = service
        .update_roadmap_details(UpdateRoadmapDetails {
            id: roadmap_id(),
            expected_version: AggregateVersion::initial(),
            name: short("Outcome Roadmap Revised"),
            details: long("Synthetic sequence with a revised outcome."),
            classification: None,
            context: context("update-roadmap"),
        })
        .unwrap_or_else(|error| panic!("update roadmap: {error}"));
    assert_eq!(roadmap.record.version.get(), 2);
    assert_eq!(service.inspect_roadmap(&roadmap_id()), Some(roadmap.record));
    assert_eq!(service.audit_events().len(), 6);
    assert!(service.audit_events().iter().all(|event| {
        event.module() == AuditModule::Portfolio
            && event.actor() == AuditActor::HeadOfProducts
            && event.occurred_at() == UtcTimestamp::from_unix_millis(1_908_072_600_000)
            && event.execution_outcome() == AuditExecutionOutcome::Succeeded
            && event.actual_effects().len() == 1
    }));
    assert_eq!(
        service
            .audit_events()
            .iter()
            .map(|event| event.id().as_str())
            .collect::<Vec<_>>(),
        vec!["audit-1", "audit-2", "audit-3", "audit-4", "audit-5", "audit-6"]
    );
}

#[test]
fn kpi_definition_and_observation_retain_decision_relevant_semantics() {
    let mut service = service();
    service
        .create_kpi_definition(CreateKpiDefinition {
            id: kpi_id(),
            name: short("Qualified adoption"),
            definition: long("Count of synthetic teams completing the public-safe workflow."),
            owner: short("Synthetic Product Lead"),
            target: short("At least 8 teams"),
            cadence: short("Weekly on Monday"),
            source: long("Synthetic evidence register"),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
            context: context("create-kpi"),
        })
        .unwrap_or_else(|error| panic!("create KPI: {error}"));

    let kpi = service
        .update_kpi_definition_details(UpdateKpiDefinitionDetails {
            id: kpi_id(),
            expected_version: AggregateVersion::initial(),
            name: short("Qualified weekly adoption"),
            definition: long("Count of synthetic teams completing the validated workflow."),
            owner: short("Synthetic Portfolio Lead"),
            target: short("At least 10 teams"),
            cadence: short("Every Monday"),
            source: long("Synthetic validated evidence register"),
            classification: None,
            context: context("update-kpi"),
        })
        .unwrap_or_else(|error| panic!("update KPI: {error}"));
    assert_eq!(kpi.record.owner.as_str(), "Synthetic Portfolio Lead");
    assert_eq!(kpi.record.target.as_str(), "At least 10 teams");
    assert_eq!(kpi.record.cadence.as_str(), "Every Monday");
    assert_eq!(
        kpi.record.source.as_str(),
        "Synthetic validated evidence register"
    );
    assert_eq!(kpi.record.version.get(), 2);

    service
        .create_kpi_observation(CreateKpiObservation {
            id: observation_id(),
            kpi_id: kpi_id(),
            value: short("7 teams"),
            observed_at: UtcTimestamp::from_unix_millis(1_908_072_600_000),
            source: long("Synthetic evidence record evidence-007"),
            classification: None,
            provenance: provenance(),
            context: context("create-observation"),
        })
        .unwrap_or_else(|error| panic!("create observation: {error}"));
    assert_eq!(
        service
            .inspect_kpi_observation(&observation_id())
            .map(|record| record.classification),
        Some(DataClassification::Public)
    );
    let observation = service
        .update_kpi_observation_details(UpdateKpiObservationDetails {
            id: observation_id(),
            expected_version: AggregateVersion::initial(),
            value: short("8 teams"),
            observed_at: UtcTimestamp::from_unix_millis(1_908_159_000_000),
            source: long("Synthetic evidence record evidence-008"),
            classification: None,
            context: context("update-observation"),
        })
        .unwrap_or_else(|error| panic!("update observation: {error}"));
    assert_eq!(observation.record.kpi_id, kpi_id());
    assert_eq!(observation.record.value.as_str(), "8 teams");
    assert_eq!(
        observation.record.observed_at.unix_millis(),
        1_908_159_000_000
    );
    assert_eq!(
        observation.record.source.as_str(),
        "Synthetic evidence record evidence-008"
    );
    assert_eq!(service.inspect_kpi_definition(&kpi_id()), Some(kpi.record));
    assert_eq!(
        service.inspect_kpi_observation(&observation_id()),
        Some(observation.record)
    );
    assert!(matches!(
        service.audit_events().last().map(|event| event.target()),
        Some(AuditTarget::KpiObservation(id)) if id == &observation_id()
    ));
}

#[test]
fn idempotency_replays_original_outcome_without_duplicate_effect() {
    let mut service = service();
    let intent = CreatePortfolio {
        id: portfolio_id(),
        name: short("Synthetic Product Portfolio"),
        details: long("Public-safe portfolio."),
        classification: Some(DataClassification::Public),
        provenance: provenance(),
        context: context("idem-create-portfolio"),
    };

    let first = service
        .create_portfolio(intent.clone())
        .unwrap_or_else(|error| panic!("first create: {error}"));
    let replay = service
        .create_portfolio(intent)
        .unwrap_or_else(|error| panic!("idempotent replay: {error}"));

    assert_eq!(replay, first);
    assert_eq!(service.audit_events().len(), 1);
    assert_eq!(
        service.inspect_portfolio(&portfolio_id()),
        Some(first.record.clone())
    );

    let duplicate = service.create_portfolio(CreatePortfolio {
        id: portfolio_id(),
        name: short("Conflicting Duplicate"),
        details: long("A second command must not replace the authoritative record."),
        classification: Some(DataClassification::Restricted),
        provenance: provenance(),
        context: context("different-idempotency"),
    });
    assert_eq!(
        duplicate.err().map(|error| error.code()),
        Some(ErrorCode::DomainConflict)
    );
    assert_eq!(service.audit_events().len(), 1);
    assert_eq!(
        service.inspect_portfolio(&portfolio_id()),
        Some(first.record)
    );

    let changed_payload = service.create_portfolio(CreatePortfolio {
        id: portfolio_id(),
        name: short("Changed Retry Payload"),
        details: long("Same identifier with changed authoritative payload."),
        classification: Some(DataClassification::Public),
        provenance: provenance(),
        context: context("idem-create-portfolio"),
    });
    let changed_error = changed_payload
        .err()
        .unwrap_or_else(|| panic!("changed idempotent payload must conflict"));
    assert_eq!(changed_error.code(), ErrorCode::DomainIdempotencyConflict);
    assert_eq!(
        changed_error.correlation_id().as_str(),
        "correlation-idem-create-portfolio"
    );

    let cross_intent = service.update_portfolio_details(UpdatePortfolioDetails {
        id: portfolio_id(),
        expected_version: AggregateVersion::initial(),
        name: short("Cross Intent Collision"),
        details: long("Create identifier cannot be reused for update."),
        classification: None,
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("idem-create-portfolio")
                .unwrap_or_else(|error| panic!("synthetic idempotency id: {error}")),
            correlation_id: CorrelationId::parse("correlation-current-attempt")
                .unwrap_or_else(|error| panic!("synthetic correlation id: {error}")),
        },
    });
    let cross_error = cross_intent
        .err()
        .unwrap_or_else(|| panic!("cross-intent idempotency reuse must conflict"));
    assert_eq!(cross_error.code(), ErrorCode::DomainIdempotencyConflict);
    assert_eq!(
        cross_error.correlation_id().as_str(),
        "correlation-current-attempt"
    );
}

#[test]
fn ordinary_and_observation_classification_cannot_be_lowered() {
    let mut service = service();
    service
        .create_product(CreateProduct {
            id: product_id(),
            name: short("Default Classified Product"),
            details: long("Synthetic record proving the fail-closed default."),
            classification: None,
            provenance: provenance(),
            context: context("create-default-product"),
        })
        .unwrap_or_else(|error| panic!("create default product: {error}"));
    assert_eq!(
        service
            .inspect_product(&product_id())
            .map(|record| record.classification),
        Some(DataClassification::Unclassified)
    );
    service
        .create_portfolio(CreatePortfolio {
            id: portfolio_id(),
            name: short("Restricted Synthetic Portfolio"),
            details: long("Synthetic restricted classification boundary."),
            classification: Some(DataClassification::Restricted),
            provenance: provenance(),
            context: context("create-restricted-portfolio"),
        })
        .unwrap_or_else(|error| panic!("create restricted portfolio: {error}"));
    let downgrade = service.update_portfolio_details(UpdatePortfolioDetails {
        id: portfolio_id(),
        expected_version: AggregateVersion::initial(),
        name: short("Forbidden Downgrade"),
        details: long("Must remain without authoritative effect."),
        classification: Some(DataClassification::Public),
        context: context("downgrade-portfolio"),
    });
    assert_eq!(
        downgrade.err().map(|error| error.code()),
        Some(ErrorCode::SecurityPolicyDenied)
    );
    assert_eq!(
        service
            .inspect_portfolio(&portfolio_id())
            .map(|record| (record.classification, record.version)),
        Some((DataClassification::Restricted, AggregateVersion::initial()))
    );

    service
        .create_kpi_definition(CreateKpiDefinition {
            id: kpi_id(),
            name: short("Restricted KPI"),
            definition: long("Synthetic restricted KPI definition."),
            owner: short("Synthetic Owner"),
            target: short("10"),
            cadence: short("Weekly"),
            source: long("Synthetic restricted evidence source"),
            classification: Some(DataClassification::Restricted),
            provenance: provenance(),
            context: context("create-restricted-kpi"),
        })
        .unwrap_or_else(|error| panic!("create restricted KPI: {error}"));
    service
        .create_kpi_observation(CreateKpiObservation {
            id: observation_id(),
            kpi_id: kpi_id(),
            value: short("7"),
            observed_at: UtcTimestamp::from_unix_millis(1_908_072_600_000),
            source: long("Synthetic restricted observation source"),
            classification: None,
            provenance: provenance(),
            context: context("create-inherited-observation"),
        })
        .unwrap_or_else(|error| panic!("create inherited observation: {error}"));
    assert_eq!(
        service
            .inspect_kpi_observation(&observation_id())
            .map(|record| record.classification),
        Some(DataClassification::Restricted)
    );
    let observation_downgrade =
        service.update_kpi_observation_details(UpdateKpiObservationDetails {
            id: observation_id(),
            expected_version: AggregateVersion::initial(),
            value: short("8"),
            observed_at: UtcTimestamp::from_unix_millis(1_908_159_000_000),
            source: long("Synthetic attempted downgrade source"),
            classification: Some(DataClassification::Public),
            context: context("downgrade-observation"),
        });
    assert_eq!(
        observation_downgrade.err().map(|error| error.code()),
        Some(ErrorCode::SecurityPolicyDenied)
    );
    assert_eq!(
        service
            .inspect_kpi_observation(&observation_id())
            .map(|record| record.version),
        Some(AggregateVersion::initial())
    );
}

#[test]
fn stale_version_and_repository_failure_have_zero_authoritative_effect() {
    let mut service = service();
    service
        .create_portfolio(CreatePortfolio {
            id: portfolio_id(),
            name: short("Synthetic Product Portfolio"),
            details: long("Original public-safe details."),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
            context: context("create-before-failures"),
        })
        .unwrap_or_else(|error| panic!("create baseline: {error}"));
    let baseline = service.inspect_portfolio(&portfolio_id());

    let stale = service.update_portfolio_details(UpdatePortfolioDetails {
        id: portfolio_id(),
        expected_version: AggregateVersion::new(2)
            .unwrap_or_else(|error| panic!("synthetic version: {error}")),
        name: short("Must Not Commit"),
        details: long("Stale write."),
        classification: Some(DataClassification::Restricted),
        context: context("stale-update"),
    });
    assert_eq!(
        stale.err().map(|error| error.code()),
        Some(ErrorCode::DomainConflict)
    );
    assert_eq!(service.inspect_portfolio(&portfolio_id()), baseline);
    assert_eq!(service.audit_events().len(), 1);

    service.inject_next_commit_failure();
    let failed = service.update_portfolio_details(UpdatePortfolioDetails {
        id: portfolio_id(),
        expected_version: AggregateVersion::initial(),
        name: short("Must Also Not Commit"),
        details: long("Repository failure write."),
        classification: Some(DataClassification::Restricted),
        context: context("repository-failure"),
    });
    assert_eq!(
        failed.err().map(|error| error.code()),
        Some(ErrorCode::PlatformInternal)
    );
    assert_eq!(service.inspect_portfolio(&portfolio_id()), baseline);
    assert_eq!(service.audit_events().len(), 1);
}
