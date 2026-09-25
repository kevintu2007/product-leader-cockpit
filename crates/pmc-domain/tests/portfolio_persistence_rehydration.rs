use pmc_domain::audit::AuditEventIdSource;
use pmc_domain::classification::DataClassification;
use pmc_domain::identity::{
    AggregateVersion, AuditEventId, CorrelationId, IdempotencyId, KpiId, KpiObservationId,
    PortfolioId, ProductId, RoadmapId,
};
use pmc_domain::portfolio::*;
use pmc_domain::provenance::{Provenance, ProvenanceReference};
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::DomainValueError;
use std::cell::Cell;

#[derive(Clone, Copy)]
struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(10)
    }
}
#[derive(Default)]
struct AuditIds(u64);
impl AuditEventIdSource for AuditIds {
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        self.0 += 1;
        AuditEventId::parse(format!("portfolio-rehydrate-audit-{}", self.0))
    }
}

struct FailingAuditIds {
    next: u64,
    fail_at: Option<u64>,
}
impl AuditEventIdSource for FailingAuditIds {
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        self.next += 1;
        if self.fail_at == Some(self.next) {
            return AuditEventId::parse("");
        }
        AuditEventId::parse(format!("fallible-portfolio-audit-{}", self.next))
    }
}

struct DecreasingClock {
    index: Cell<usize>,
    values: Vec<i64>,
}
impl Clock for DecreasingClock {
    fn now(&self) -> UtcTimestamp {
        let index = self.index.get();
        self.index.set(index + 1);
        UtcTimestamp::from_unix_millis(self.values[index])
    }
}
fn context(id: &str, correlation: &str) -> OperationContext {
    OperationContext {
        idempotency_id: IdempotencyId::parse(id).unwrap(),
        correlation_id: CorrelationId::parse(correlation).unwrap(),
    }
}
fn short(v: &str) -> ShortText {
    ShortText::parse(v).unwrap()
}
fn long(v: &str) -> LongText {
    LongText::parse(v).unwrap()
}
fn provenance() -> Provenance {
    Provenance::SyntheticFixture(ProvenanceReference::parse("portfolio-rehydrate").unwrap())
}

#[test]
fn validated_snapshot_rehydrates_and_exactly_replays_original_outcome() {
    let mut original = InMemoryPortfolioService::new(FixedClock, AuditIds::default());
    let command = CreatePortfolio {
        id: PortfolioId::parse("portfolio-rehydrate").unwrap(),
        name: short("Synthetic portfolio"),
        details: long("Synthetic details"),
        classification: Some(DataClassification::Internal),
        provenance: provenance(),
        context: context("portfolio-create", "original-correlation"),
    };
    let expected = original.create_portfolio(command.clone()).unwrap();
    let snapshot = original.persistence_snapshot();
    let validated = PortfolioPersistenceSnapshot::validate(
        snapshot.portfolios().to_vec(),
        snapshot.products().to_vec(),
        snapshot.roadmaps().to_vec(),
        snapshot.kpi_definitions().to_vec(),
        snapshot.kpi_observations().to_vec(),
        snapshot.replay().to_vec(),
        snapshot.audits().to_vec(),
    )
    .unwrap();
    let mut restored =
        InMemoryPortfolioService::rehydrate(FixedClock, AuditIds::default(), validated);
    let audit_count = restored.audit_events().len();
    let replayed = restored
        .create_portfolio(CreatePortfolio {
            context: context("portfolio-create", "new-correlation"),
            ..command
        })
        .unwrap();
    assert_eq!(replayed, expected);
    assert_eq!(restored.audit_events().len(), audit_count);
}

#[test]
fn every_portfolio_command_survives_validated_rehydration_with_exact_history() {
    let mut service = InMemoryPortfolioService::new(FixedClock, AuditIds::default());
    let portfolio = PortfolioId::parse("all-portfolio").unwrap();
    let product = ProductId::parse("all-product").unwrap();
    let roadmap = RoadmapId::parse("all-roadmap").unwrap();
    let kpi = KpiId::parse("all-kpi").unwrap();
    let observation = KpiObservationId::parse("all-observation").unwrap();
    service
        .create_portfolio(CreatePortfolio {
            id: portfolio.clone(),
            name: short("Portfolio 1"),
            details: long("Portfolio details 1"),
            classification: Some(DataClassification::Internal),
            provenance: provenance(),
            context: context("p1", "c-p1"),
        })
        .unwrap();
    service
        .update_portfolio_details(UpdatePortfolioDetails {
            id: portfolio,
            expected_version: AggregateVersion::initial(),
            name: short("Portfolio 2"),
            details: long("Portfolio details 2"),
            classification: Some(DataClassification::Confidential),
            context: context("p2", "c-p2"),
        })
        .unwrap();
    service
        .create_product(CreateProduct {
            id: product.clone(),
            name: short("Product 1"),
            details: long("Product details 1"),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
            context: context("d1", "c-d1"),
        })
        .unwrap();
    service
        .update_product_details(UpdateProductDetails {
            id: product,
            expected_version: AggregateVersion::initial(),
            name: short("Product 2"),
            details: long("Product details 2"),
            classification: None,
            context: context("d2", "c-d2"),
        })
        .unwrap();
    service
        .create_roadmap(CreateRoadmap {
            id: roadmap.clone(),
            name: short("Roadmap 1"),
            details: long("Roadmap details 1"),
            classification: Some(DataClassification::Internal),
            provenance: provenance(),
            context: context("r1", "c-r1"),
        })
        .unwrap();
    service
        .update_roadmap_details(UpdateRoadmapDetails {
            id: roadmap,
            expected_version: AggregateVersion::initial(),
            name: short("Roadmap 2"),
            details: long("Roadmap details 2"),
            classification: None,
            context: context("r2", "c-r2"),
        })
        .unwrap();
    service
        .create_kpi_definition(CreateKpiDefinition {
            id: kpi.clone(),
            name: short("KPI 1"),
            definition: long("Definition 1"),
            owner: short("Owner 1"),
            target: short("Target 1"),
            cadence: short("Weekly"),
            source: long("Source 1"),
            classification: Some(DataClassification::Internal),
            provenance: provenance(),
            context: context("k1", "c-k1"),
        })
        .unwrap();
    service
        .update_kpi_definition_details(UpdateKpiDefinitionDetails {
            id: kpi.clone(),
            expected_version: AggregateVersion::initial(),
            name: short("KPI 2"),
            definition: long("Definition 2"),
            owner: short("Owner 2"),
            target: short("Target 2"),
            cadence: short("Monthly"),
            source: long("Source 2"),
            classification: Some(DataClassification::Confidential),
            context: context("k2", "c-k2"),
        })
        .unwrap();
    service
        .create_kpi_observation(CreateKpiObservation {
            id: observation.clone(),
            kpi_id: kpi,
            value: short("42"),
            observed_at: UtcTimestamp::from_unix_millis(11),
            source: long("Observation source 1"),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
            context: context("o1", "c-o1"),
        })
        .unwrap();
    let final_outcome = service
        .update_kpi_observation_details(UpdateKpiObservationDetails {
            id: observation.clone(),
            expected_version: AggregateVersion::initial(),
            value: short("43"),
            observed_at: UtcTimestamp::from_unix_millis(12),
            source: long("Observation source 2"),
            classification: Some(DataClassification::Restricted),
            context: context("o2", "c-o2"),
        })
        .unwrap();
    let snapshot = service.persistence_snapshot();
    assert_eq!(snapshot.replay().len(), 10);
    let validated = PortfolioPersistenceSnapshot::validate(
        snapshot.portfolios().to_vec(),
        snapshot.products().to_vec(),
        snapshot.roadmaps().to_vec(),
        snapshot.kpi_definitions().to_vec(),
        snapshot.kpi_observations().to_vec(),
        snapshot.replay().to_vec(),
        snapshot.audits().to_vec(),
    )
    .unwrap();
    let mut restored =
        InMemoryPortfolioService::rehydrate(FixedClock, AuditIds::default(), validated);
    let replayed = restored
        .update_kpi_observation_details(UpdateKpiObservationDetails {
            id: observation,
            expected_version: AggregateVersion::initial(),
            value: short("43"),
            observed_at: UtcTimestamp::from_unix_millis(12),
            source: long("Observation source 2"),
            classification: Some(DataClassification::Restricted),
            context: context("o2", "different-correlation"),
        })
        .unwrap();
    assert_eq!(replayed, final_outcome);
    assert_eq!(restored.audit_events().len(), 10);
}

#[test]
fn validation_rejects_ordinal_gaps_and_reordered_audits() {
    let mut service = InMemoryPortfolioService::new(FixedClock, AuditIds::default());
    service
        .create_portfolio(CreatePortfolio {
            id: PortfolioId::parse("tamper-one").unwrap(),
            name: short("One"),
            details: long("One details"),
            classification: None,
            provenance: provenance(),
            context: context("t1", "tc1"),
        })
        .unwrap();
    service
        .create_product(CreateProduct {
            id: ProductId::parse("tamper-two").unwrap(),
            name: short("Two"),
            details: long("Two details"),
            classification: None,
            provenance: provenance(),
            context: context("t2", "tc2"),
        })
        .unwrap();
    let snapshot = service.persistence_snapshot();
    let first = &snapshot.replay()[0];
    let gap = PortfolioReplayCapsule::new(
        first.idempotency_id().clone(),
        first.command().clone(),
        first.result().clone(),
        first.correlation_id().clone(),
        first.audit_event_ids().to_vec(),
        4,
        first.derived_kpi_observation_mutations().to_vec(),
    );
    let mut replay = snapshot.replay().to_vec();
    replay[0] = gap;
    assert_eq!(
        PortfolioPersistenceSnapshot::validate(
            snapshot.portfolios().to_vec(),
            snapshot.products().to_vec(),
            snapshot.roadmaps().to_vec(),
            snapshot.kpi_definitions().to_vec(),
            snapshot.kpi_observations().to_vec(),
            replay,
            snapshot.audits().to_vec()
        )
        .unwrap_err(),
        PortfolioRehydrationError::VersionLineageMismatch
    );
    let mut audits = snapshot.audits().to_vec();
    audits.reverse();
    assert_eq!(
        PortfolioPersistenceSnapshot::validate(
            snapshot.portfolios().to_vec(),
            snapshot.products().to_vec(),
            snapshot.roadmaps().to_vec(),
            snapshot.kpi_definitions().to_vec(),
            snapshot.kpi_observations().to_vec(),
            snapshot.replay().to_vec(),
            audits
        )
        .unwrap_err(),
        PortfolioRehydrationError::AuditMismatch
    );
}

#[test]
fn validation_rejects_hidden_lowering_duplicate_records_and_missing_kpi_parent() {
    let mut service = InMemoryPortfolioService::new(FixedClock, AuditIds::default());
    let portfolio_id = PortfolioId::parse("restricted-portfolio").unwrap();
    service
        .create_portfolio(CreatePortfolio {
            id: portfolio_id.clone(),
            name: short("Restricted 1"),
            details: long("Restricted details 1"),
            classification: Some(DataClassification::Internal),
            provenance: provenance(),
            context: context("lower-1", "lower-c1"),
        })
        .unwrap();
    service
        .update_portfolio_details(UpdatePortfolioDetails {
            id: portfolio_id.clone(),
            expected_version: AggregateVersion::initial(),
            name: short("Restricted 2"),
            details: long("Restricted details 2"),
            classification: Some(DataClassification::Restricted),
            context: context("lower-2", "lower-c2"),
        })
        .unwrap();
    let snapshot = service.persistence_snapshot();
    let mut replay = snapshot.replay().to_vec();
    let update = replay
        .iter_mut()
        .find(|v| matches!(v.command(), CommandIdentity::UpdatePortfolio { .. }))
        .unwrap();
    *update = PortfolioReplayCapsule::new(
        update.idempotency_id().clone(),
        CommandIdentity::UpdatePortfolio {
            id: portfolio_id,
            expected_version: AggregateVersion::initial(),
            name: short("Restricted 2"),
            details: long("Restricted details 2"),
            classification: Some(DataClassification::Public),
        },
        update.result().clone(),
        update.correlation_id().clone(),
        update.audit_event_ids().to_vec(),
        update.operation_ordinal(),
        update.derived_kpi_observation_mutations().to_vec(),
    );
    assert_eq!(
        PortfolioPersistenceSnapshot::validate(
            snapshot.portfolios().to_vec(),
            snapshot.products().to_vec(),
            snapshot.roadmaps().to_vec(),
            snapshot.kpi_definitions().to_vec(),
            snapshot.kpi_observations().to_vec(),
            replay,
            snapshot.audits().to_vec()
        )
        .unwrap_err(),
        PortfolioRehydrationError::ResultMismatch
    );
    let duplicate = vec![
        snapshot.portfolios()[0].clone(),
        snapshot.portfolios()[0].clone(),
    ];
    assert_eq!(
        PortfolioPersistenceSnapshot::validate(
            duplicate,
            vec![],
            vec![],
            vec![],
            vec![],
            snapshot.replay().to_vec(),
            snapshot.audits().to_vec()
        )
        .unwrap_err(),
        PortfolioRehydrationError::DuplicateRecord
    );

    let mut kpi_service = InMemoryPortfolioService::new(FixedClock, AuditIds::default());
    let kpi = KpiId::parse("parent-kpi").unwrap();
    kpi_service
        .create_kpi_definition(CreateKpiDefinition {
            id: kpi.clone(),
            name: short("Parent KPI"),
            definition: long("Parent definition"),
            owner: short("Owner"),
            target: short("Target"),
            cadence: short("Weekly"),
            source: long("KPI source"),
            classification: None,
            provenance: provenance(),
            context: context("parent", "parent-c"),
        })
        .unwrap();
    kpi_service
        .create_kpi_observation(CreateKpiObservation {
            id: KpiObservationId::parse("orphan-observation").unwrap(),
            kpi_id: kpi,
            value: short("1"),
            observed_at: UtcTimestamp::from_unix_millis(30),
            source: long("Observation source"),
            classification: None,
            provenance: provenance(),
            context: context("child", "child-c"),
        })
        .unwrap();
    let kpi_snapshot = kpi_service.persistence_snapshot();
    assert_eq!(
        PortfolioPersistenceSnapshot::validate(
            vec![],
            vec![],
            vec![],
            vec![],
            kpi_snapshot.kpi_observations().to_vec(),
            kpi_snapshot.replay().to_vec(),
            kpi_snapshot.audits().to_vec()
        )
        .unwrap_err(),
        PortfolioRehydrationError::MissingParent
    );
}

#[test]
fn observation_update_inherits_a_more_restrictive_current_definition() {
    let mut service = InMemoryPortfolioService::new(FixedClock, AuditIds::default());
    let kpi = KpiId::parse("inheritance-kpi").unwrap();
    let observation = KpiObservationId::parse("inheritance-observation").unwrap();
    service
        .create_kpi_definition(CreateKpiDefinition {
            id: kpi.clone(),
            name: short("Inheritance KPI"),
            definition: long("Definition"),
            owner: short("Owner"),
            target: short("Target"),
            cadence: short("Weekly"),
            source: long("Definition source"),
            classification: Some(DataClassification::Internal),
            provenance: provenance(),
            context: context("inherit-k1", "inherit-c1"),
        })
        .unwrap();
    service
        .create_kpi_observation(CreateKpiObservation {
            id: observation.clone(),
            kpi_id: kpi.clone(),
            value: short("1"),
            observed_at: UtcTimestamp::from_unix_millis(40),
            source: long("Observation source 1"),
            classification: None,
            provenance: provenance(),
            context: context("inherit-o1", "inherit-c2"),
        })
        .unwrap();
    service
        .update_kpi_definition_details(UpdateKpiDefinitionDetails {
            id: kpi,
            expected_version: AggregateVersion::initial(),
            name: short("Inheritance KPI"),
            definition: long("Definition"),
            owner: short("Owner"),
            target: short("Target"),
            cadence: short("Weekly"),
            source: long("Definition source"),
            classification: Some(DataClassification::Confidential),
            context: context("inherit-k2", "inherit-c3"),
        })
        .unwrap();
    let inherited = service.inspect_kpi_observation(&observation).unwrap();
    assert_eq!(inherited.classification, DataClassification::Confidential);
    assert_eq!(inherited.version.get(), 2);
    assert_eq!(service.audit_events().len(), 4);
    let updated = service
        .update_kpi_observation_details(UpdateKpiObservationDetails {
            id: observation,
            expected_version: AggregateVersion::new(2).unwrap(),
            value: short("2"),
            observed_at: UtcTimestamp::from_unix_millis(41),
            source: long("Observation source 2"),
            classification: None,
            context: context("inherit-o2", "inherit-c4"),
        })
        .unwrap();
    assert_eq!(
        updated.record.classification,
        DataClassification::Confidential
    );
    assert_eq!(updated.record.version.get(), 3);
    let snapshot = service.persistence_snapshot();
    PortfolioPersistenceSnapshot::validate(
        snapshot.portfolios().to_vec(),
        snapshot.products().to_vec(),
        snapshot.roadmaps().to_vec(),
        snapshot.kpi_definitions().to_vec(),
        snapshot.kpi_observations().to_vec(),
        snapshot.replay().to_vec(),
        snapshot.audits().to_vec(),
    )
    .unwrap();
}

#[test]
fn fan_out_audit_id_and_commit_failures_roll_back_then_retry_exactly_once() {
    let kpi = KpiId::parse("atomic-kpi").unwrap();
    let observation = KpiObservationId::parse("atomic-observation").unwrap();
    let mut service = InMemoryPortfolioService::new(
        FixedClock,
        FailingAuditIds {
            next: 0,
            fail_at: Some(4),
        },
    );
    service
        .create_kpi_definition(CreateKpiDefinition {
            id: kpi.clone(),
            name: short("Atomic KPI"),
            definition: long("Atomic definition"),
            owner: short("Owner"),
            target: short("Target"),
            cadence: short("Weekly"),
            source: long("Definition source"),
            classification: Some(DataClassification::Internal),
            provenance: provenance(),
            context: context("atomic-k1", "atomic-c1"),
        })
        .unwrap();
    service
        .create_kpi_observation(CreateKpiObservation {
            id: observation.clone(),
            kpi_id: kpi.clone(),
            value: short("1"),
            observed_at: UtcTimestamp::from_unix_millis(50),
            source: long("Observation source"),
            classification: None,
            provenance: provenance(),
            context: context("atomic-o1", "atomic-c2"),
        })
        .unwrap();
    let raise = UpdateKpiDefinitionDetails {
        id: kpi.clone(),
        expected_version: AggregateVersion::initial(),
        name: short("Atomic KPI"),
        definition: long("Atomic definition"),
        owner: short("Owner"),
        target: short("Target"),
        cadence: short("Weekly"),
        source: long("Definition source"),
        classification: Some(DataClassification::Restricted),
        context: context("atomic-k2", "atomic-c3"),
    };
    assert!(service
        .update_kpi_definition_details(raise.clone())
        .is_err());
    assert_eq!(
        service.inspect_kpi_definition(&kpi).unwrap().version,
        AggregateVersion::initial()
    );
    assert_eq!(
        service
            .inspect_kpi_observation(&observation)
            .unwrap()
            .classification,
        DataClassification::Internal
    );
    assert_eq!(service.audit_events().len(), 2);
    service.inject_next_commit_failure();
    assert!(service
        .update_kpi_definition_details(raise.clone())
        .is_err());
    assert_eq!(
        service
            .inspect_kpi_observation(&observation)
            .unwrap()
            .version,
        AggregateVersion::initial()
    );
    assert_eq!(service.audit_events().len(), 2);
    let raised = service
        .update_kpi_definition_details(raise.clone())
        .unwrap();
    assert_eq!(raised.record.classification, DataClassification::Restricted);
    assert_eq!(
        service
            .inspect_kpi_observation(&observation)
            .unwrap()
            .classification,
        DataClassification::Restricted
    );
    assert_eq!(
        service
            .inspect_kpi_observation(&observation)
            .unwrap()
            .version
            .get(),
        2
    );
    assert_eq!(service.audit_events().len(), 4);
    let snapshot = service.persistence_snapshot();
    let validated = PortfolioPersistenceSnapshot::validate(
        snapshot.portfolios().to_vec(),
        snapshot.products().to_vec(),
        snapshot.roadmaps().to_vec(),
        snapshot.kpi_definitions().to_vec(),
        snapshot.kpi_observations().to_vec(),
        snapshot.replay().to_vec(),
        snapshot.audits().to_vec(),
    )
    .unwrap();
    let mut restored = InMemoryPortfolioService::rehydrate(
        FixedClock,
        FailingAuditIds {
            next: 50,
            fail_at: None,
        },
        validated,
    );
    let restored_audits = restored.audit_events().to_vec();
    let restored_replay = restored
        .update_kpi_definition_details(UpdateKpiDefinitionDetails {
            context: context("atomic-k2", "restored-replay-correlation"),
            ..raise.clone()
        })
        .unwrap();
    assert_eq!(restored_replay, raised);
    assert_eq!(restored.audit_events(), restored_audits);
    assert_eq!(
        restored.inspect_kpi_observation(&observation),
        service.inspect_kpi_observation(&observation)
    );
    let before_replay = service.audit_events().to_vec();
    let replayed = service
        .update_kpi_definition_details(UpdateKpiDefinitionDetails {
            context: context("atomic-k2", "different-replay-correlation"),
            ..raise
        })
        .unwrap();
    assert_eq!(replayed, raised);
    assert_eq!(service.audit_events(), before_replay);
}

#[test]
fn decreasing_wall_clock_history_is_valid_while_record_time_remains_ordered() {
    let clock = DecreasingClock {
        index: Cell::new(0),
        values: vec![30, 20, 10],
    };
    let mut service = InMemoryPortfolioService::new(clock, AuditIds::default());
    let kpi = KpiId::parse("decreasing-kpi").unwrap();
    let observation = KpiObservationId::parse("decreasing-observation").unwrap();
    service
        .create_kpi_definition(CreateKpiDefinition {
            id: kpi.clone(),
            name: short("Clock KPI"),
            definition: long("Clock definition"),
            owner: short("Owner"),
            target: short("Target"),
            cadence: short("Weekly"),
            source: long("Definition source"),
            classification: Some(DataClassification::Internal),
            provenance: provenance(),
            context: context("clock-k1", "clock-c1"),
        })
        .unwrap();
    service
        .create_kpi_observation(CreateKpiObservation {
            id: observation.clone(),
            kpi_id: kpi,
            value: short("1"),
            observed_at: UtcTimestamp::from_unix_millis(5),
            source: long("Observation source 1"),
            classification: None,
            provenance: provenance(),
            context: context("clock-o1", "clock-c2"),
        })
        .unwrap();
    service
        .update_kpi_observation_details(UpdateKpiObservationDetails {
            id: observation.clone(),
            expected_version: AggregateVersion::initial(),
            value: short("2"),
            observed_at: UtcTimestamp::from_unix_millis(6),
            source: long("Observation source 2"),
            classification: None,
            context: context("clock-o2", "clock-c3"),
        })
        .unwrap();
    assert_eq!(
        service
            .audit_events()
            .iter()
            .map(|v| v.occurred_at().unix_millis())
            .collect::<Vec<_>>(),
        vec![30, 20, 10]
    );
    let record = service.inspect_kpi_observation(&observation).unwrap();
    assert!(record.created_at <= record.updated_at);
    let snapshot = service.persistence_snapshot();
    let validated = PortfolioPersistenceSnapshot::validate(
        snapshot.portfolios().to_vec(),
        snapshot.products().to_vec(),
        snapshot.roadmaps().to_vec(),
        snapshot.kpi_definitions().to_vec(),
        snapshot.kpi_observations().to_vec(),
        snapshot.replay().to_vec(),
        snapshot.audits().to_vec(),
    )
    .unwrap();
    let restored = InMemoryPortfolioService::rehydrate(FixedClock, AuditIds::default(), validated);
    assert_eq!(restored.inspect_kpi_observation(&observation), Some(record));
}

#[test]
fn fan_out_is_sorted_exact_and_unclassified_remains_absorbing() {
    let mut service = InMemoryPortfolioService::new(FixedClock, AuditIds::default());
    let kpi = KpiId::parse("fanout-kpi").unwrap();
    service
        .create_kpi_definition(CreateKpiDefinition {
            id: kpi.clone(),
            name: short("Fanout KPI"),
            definition: long("Fanout definition"),
            owner: short("Owner"),
            target: short("Target"),
            cadence: short("Weekly"),
            source: long("Definition source"),
            classification: Some(DataClassification::Internal),
            provenance: provenance(),
            context: context("fan-k1", "fan-c1"),
        })
        .unwrap();
    for (id, classification) in [
        ("z-observation", Some(DataClassification::Internal)),
        ("a-observation", Some(DataClassification::Internal)),
        ("u-observation", Some(DataClassification::Unclassified)),
    ] {
        service
            .create_kpi_observation(CreateKpiObservation {
                id: KpiObservationId::parse(id).unwrap(),
                kpi_id: kpi.clone(),
                value: short("1"),
                observed_at: UtcTimestamp::from_unix_millis(60),
                source: long("Observation source"),
                classification,
                provenance: provenance(),
                context: context(&format!("create-{id}"), &format!("correlation-{id}")),
            })
            .unwrap();
    }
    service
        .update_kpi_definition_details(UpdateKpiDefinitionDetails {
            id: kpi,
            expected_version: AggregateVersion::initial(),
            name: short("Fanout KPI"),
            definition: long("Fanout definition"),
            owner: short("Owner"),
            target: short("Target"),
            cadence: short("Weekly"),
            source: long("Definition source"),
            classification: Some(DataClassification::Restricted),
            context: context("fan-k2", "fan-c2"),
        })
        .unwrap();
    let snapshot = service.persistence_snapshot();
    let capsule = snapshot
        .replay()
        .iter()
        .find(|v| matches!(v.command(), CommandIdentity::UpdateKpi { .. }))
        .unwrap();
    assert_eq!(
        capsule
            .derived_kpi_observation_mutations()
            .iter()
            .map(|v| v.observation_id().as_str())
            .collect::<Vec<_>>(),
        vec!["a-observation", "z-observation"]
    );
    assert_eq!(
        service
            .inspect_kpi_observation(&KpiObservationId::parse("u-observation").unwrap())
            .unwrap()
            .classification,
        DataClassification::Unclassified
    );
    let mut reversed_mutations = capsule.derived_kpi_observation_mutations().to_vec();
    reversed_mutations.reverse();
    let tampered = PortfolioReplayCapsule::new(
        capsule.idempotency_id().clone(),
        capsule.command().clone(),
        capsule.result().clone(),
        capsule.correlation_id().clone(),
        capsule.audit_event_ids().to_vec(),
        capsule.operation_ordinal(),
        reversed_mutations,
    );
    let mut replay = snapshot.replay().to_vec();
    let index = replay
        .iter()
        .position(|v| v.operation_ordinal() == capsule.operation_ordinal())
        .unwrap();
    replay[index] = tampered;
    assert!(PortfolioPersistenceSnapshot::validate(
        snapshot.portfolios().to_vec(),
        snapshot.products().to_vec(),
        snapshot.roadmaps().to_vec(),
        snapshot.kpi_definitions().to_vec(),
        snapshot.kpi_observations().to_vec(),
        replay,
        snapshot.audits().to_vec()
    )
    .is_err());
}
