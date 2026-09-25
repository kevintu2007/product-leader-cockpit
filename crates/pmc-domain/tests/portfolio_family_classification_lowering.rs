//! H2a "Lower Data Classification" for the other Portfolio-family record
//! types (Product, Roadmap, Kpi, KpiObservation) --
//! the same operation `portfolio_classification_lowering.rs` proved for
//! Portfolio itself, mechanically extended.
//!
//! The shared H2a machinery (digest binding, TTL expiry, actor
//! authorization, replay/idempotency) is already exhaustively covered by
//! `portfolio_classification_lowering.rs` and the cross-cutting
//! `h2a_operation_conformance.rs` suite. Each type here gets a narrower set
//! proving *its own* wiring is correct: happy path (with the right audit
//! target), a stale-version rejection, and a not-a-genuine-lowering
//! rejection -- not a full re-run of every shared-primitive edge case.

use pmc_domain::audit::{AuditActor, AuditEventIdSource, AuditTarget};
use pmc_domain::classification::DataClassification;
use pmc_domain::error::ErrorCode;
use pmc_domain::identity::{
    AggregateVersion, ApprovalReceiptId, AuditEventId, CorrelationId, IdempotencyId, KpiId,
    KpiObservationId, PreparedIntentId, ProductId, RoadmapId,
};
use pmc_domain::portfolio::*;
use pmc_domain::provenance::{Provenance, ProvenanceReference};
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::work_management::{
    ApprovalAuthorizationPort, ApprovalConfirmation, WorkManagementApproval,
    WorkManagementPreparedIntent,
};
use pmc_domain::DomainValueError;

#[derive(Clone, Copy)]
struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(1_000)
    }
}

struct SequentialAuditIds(u64);
impl AuditEventIdSource for SequentialAuditIds {
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        self.0 += 1;
        AuditEventId::parse(format!("audit-{}", self.0))
    }
}

struct SequentialIds(u64);
impl PortfolioClassificationLoweringIdSource for SequentialIds {
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        self.0 += 1;
        PreparedIntentId::parse(format!("prepared-{}", self.0))
    }
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError> {
        self.0 += 1;
        ApprovalReceiptId::parse(format!("receipt-{}", self.0))
    }
}

#[derive(Clone, Copy)]
struct AllowHeadOfProducts;
impl ApprovalAuthorizationPort for AllowHeadOfProducts {
    fn authorize(&self, actor: AuditActor) -> bool {
        actor == AuditActor::HeadOfProducts
    }
}

fn service() -> InMemoryPortfolioService<FixedClock, SequentialAuditIds> {
    InMemoryPortfolioService::new(FixedClock, SequentialAuditIds(0))
}

fn short(value: &str) -> ShortText {
    ShortText::parse(value).unwrap()
}
fn long(value: &str) -> LongText {
    LongText::parse(value).unwrap()
}
fn rationale(value: &str) -> pmc_domain::work_management::WorkManagementRationale {
    pmc_domain::work_management::WorkManagementRationale::parse(value).unwrap()
}
fn context(id: &str) -> OperationContext {
    OperationContext {
        idempotency_id: IdempotencyId::parse(id).unwrap(),
        correlation_id: CorrelationId::parse(format!("correlation-{id}")).unwrap(),
    }
}
fn provenance() -> Provenance {
    Provenance::SyntheticFixture(
        ProvenanceReference::parse("synthetic-portfolio-family-lowering-v1").unwrap(),
    )
}
fn approval(prepared: &WorkManagementPreparedIntent, key: &str) -> WorkManagementApproval {
    WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse(key).unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap()
}

fn product_id() -> ProductId {
    ProductId::parse("product-lowering").unwrap()
}

fn create_restricted_product(
    service: &mut InMemoryPortfolioService<FixedClock, SequentialAuditIds>,
) -> ProductRecord {
    service
        .create_product(CreateProduct {
            id: product_id(),
            name: short("Synthetic Product"),
            details: long("Public-safe product used only by deterministic tests."),
            classification: Some(DataClassification::Restricted),
            provenance: provenance(),
            context: context("create-product"),
        })
        .unwrap()
        .record
}

#[test]
fn product_prepare_then_approve_atomically_lowers_classification() {
    let mut service = service();
    let mut ids = SequentialIds(0);
    let product = create_restricted_product(&mut service);

    let prepared = service
        .prepare_lower_product_classification(
            PrepareLowerProductClassification {
                product_id: product.id.clone(),
                expected_version: product.version,
                proposed_classification: DataClassification::Internal,
                rationale: rationale("Synthetic rationale."),
                context: context("product-lower-prepare"),
            },
            &mut ids,
        )
        .unwrap_or_else(|error| panic!("prepare: {error}"));

    let outcome = service
        .approve_and_execute_lower_product_classification(
            ApproveAndExecuteLowerProductClassification {
                approval: approval(&prepared, "product-lower-approve"),
                context: context("product-lower-approve"),
            },
            &mut ids,
            &AllowHeadOfProducts,
        )
        .unwrap_or_else(|error| panic!("approve: {error}"));

    assert_eq!(outcome.record.classification, DataClassification::Internal);
    assert_eq!(outcome.record.version, product.version.next().unwrap());
    assert!(matches!(
        service.audit_events().last().map(|event| event.target()),
        Some(AuditTarget::Product(id)) if id == &product_id()
    ));
}

#[test]
fn product_prepare_rejects_a_stale_version_and_a_non_lowering_proposal() {
    let mut service = service();
    let mut ids = SequentialIds(0);
    let product = create_restricted_product(&mut service);

    let stale = service
        .prepare_lower_product_classification(
            PrepareLowerProductClassification {
                product_id: product.id.clone(),
                expected_version: AggregateVersion::initial().next().unwrap(),
                proposed_classification: DataClassification::Internal,
                rationale: rationale("Synthetic rationale."),
                context: context("product-stale"),
            },
            &mut ids,
        )
        .unwrap_err();
    assert_eq!(stale.code(), ErrorCode::DomainConflict);

    let unchanged = service
        .prepare_lower_product_classification(
            PrepareLowerProductClassification {
                product_id: product.id.clone(),
                expected_version: product.version,
                proposed_classification: DataClassification::Restricted,
                rationale: rationale("Synthetic rationale."),
                context: context("product-unchanged"),
            },
            &mut ids,
        )
        .unwrap_err();
    assert_eq!(unchanged.code(), ErrorCode::DomainConflict);
}

fn roadmap_id() -> RoadmapId {
    RoadmapId::parse("roadmap-lowering").unwrap()
}

fn create_restricted_roadmap(
    service: &mut InMemoryPortfolioService<FixedClock, SequentialAuditIds>,
) -> RoadmapRecord {
    service
        .create_roadmap(CreateRoadmap {
            id: roadmap_id(),
            name: short("Synthetic Roadmap"),
            details: long("Public-safe roadmap used only by deterministic tests."),
            classification: Some(DataClassification::Restricted),
            provenance: provenance(),
            context: context("create-roadmap"),
        })
        .unwrap()
        .record
}

#[test]
fn roadmap_prepare_then_approve_atomically_lowers_classification() {
    let mut service = service();
    let mut ids = SequentialIds(0);
    let roadmap = create_restricted_roadmap(&mut service);

    let prepared = service
        .prepare_lower_roadmap_classification(
            PrepareLowerRoadmapClassification {
                roadmap_id: roadmap.id.clone(),
                expected_version: roadmap.version,
                proposed_classification: DataClassification::Internal,
                rationale: rationale("Synthetic rationale."),
                context: context("roadmap-lower-prepare"),
            },
            &mut ids,
        )
        .unwrap_or_else(|error| panic!("prepare: {error}"));

    let outcome = service
        .approve_and_execute_lower_roadmap_classification(
            ApproveAndExecuteLowerRoadmapClassification {
                approval: approval(&prepared, "roadmap-lower-approve"),
                context: context("roadmap-lower-approve"),
            },
            &mut ids,
            &AllowHeadOfProducts,
        )
        .unwrap_or_else(|error| panic!("approve: {error}"));

    assert_eq!(outcome.record.classification, DataClassification::Internal);
    assert_eq!(outcome.record.version, roadmap.version.next().unwrap());
    assert!(matches!(
        service.audit_events().last().map(|event| event.target()),
        Some(AuditTarget::Roadmap(id)) if id == &roadmap_id()
    ));
}

#[test]
fn roadmap_prepare_rejects_a_stale_version_and_a_non_lowering_proposal() {
    let mut service = service();
    let mut ids = SequentialIds(0);
    let roadmap = create_restricted_roadmap(&mut service);

    let stale = service
        .prepare_lower_roadmap_classification(
            PrepareLowerRoadmapClassification {
                roadmap_id: roadmap.id.clone(),
                expected_version: AggregateVersion::initial().next().unwrap(),
                proposed_classification: DataClassification::Internal,
                rationale: rationale("Synthetic rationale."),
                context: context("roadmap-stale"),
            },
            &mut ids,
        )
        .unwrap_err();
    assert_eq!(stale.code(), ErrorCode::DomainConflict);

    let unchanged = service
        .prepare_lower_roadmap_classification(
            PrepareLowerRoadmapClassification {
                roadmap_id: roadmap.id.clone(),
                expected_version: roadmap.version,
                proposed_classification: DataClassification::Restricted,
                rationale: rationale("Synthetic rationale."),
                context: context("roadmap-unchanged"),
            },
            &mut ids,
        )
        .unwrap_err();
    assert_eq!(unchanged.code(), ErrorCode::DomainConflict);
}

fn kpi_id() -> KpiId {
    KpiId::parse("kpi-lowering").unwrap()
}

fn create_restricted_kpi(
    service: &mut InMemoryPortfolioService<FixedClock, SequentialAuditIds>,
) -> KpiDefinitionRecord {
    service
        .create_kpi_definition(CreateKpiDefinition {
            id: kpi_id(),
            name: short("Synthetic KPI"),
            definition: long("Public-safe KPI definition used only by deterministic tests."),
            owner: short("Synthetic Owner"),
            target: short("Synthetic Target"),
            cadence: short("Monthly"),
            source: long("Synthetic source system."),
            classification: Some(DataClassification::Restricted),
            provenance: provenance(),
            context: context("create-kpi"),
        })
        .unwrap()
        .record
}

#[test]
fn kpi_prepare_then_approve_atomically_lowers_classification() {
    let mut service = service();
    let mut ids = SequentialIds(0);
    let kpi = create_restricted_kpi(&mut service);

    let prepared = service
        .prepare_lower_kpi_classification(
            PrepareLowerKpiClassification {
                kpi_id: kpi.id.clone(),
                expected_version: kpi.version,
                proposed_classification: DataClassification::Internal,
                rationale: rationale("Synthetic rationale."),
                context: context("kpi-lower-prepare"),
            },
            &mut ids,
        )
        .unwrap_or_else(|error| panic!("prepare: {error}"));

    let outcome = service
        .approve_and_execute_lower_kpi_classification(
            ApproveAndExecuteLowerKpiClassification {
                approval: approval(&prepared, "kpi-lower-approve"),
                context: context("kpi-lower-approve"),
            },
            &mut ids,
            &AllowHeadOfProducts,
        )
        .unwrap_or_else(|error| panic!("approve: {error}"));

    assert_eq!(outcome.record.classification, DataClassification::Internal);
    assert_eq!(outcome.record.version, kpi.version.next().unwrap());
    assert!(matches!(
        service.audit_events().last().map(|event| event.target()),
        Some(AuditTarget::Kpi(id)) if id == &kpi_id()
    ));
}

#[test]
fn kpi_prepare_rejects_a_stale_version_and_a_non_lowering_proposal() {
    let mut service = service();
    let mut ids = SequentialIds(0);
    let kpi = create_restricted_kpi(&mut service);

    let stale = service
        .prepare_lower_kpi_classification(
            PrepareLowerKpiClassification {
                kpi_id: kpi.id.clone(),
                expected_version: AggregateVersion::initial().next().unwrap(),
                proposed_classification: DataClassification::Internal,
                rationale: rationale("Synthetic rationale."),
                context: context("kpi-stale"),
            },
            &mut ids,
        )
        .unwrap_err();
    assert_eq!(stale.code(), ErrorCode::DomainConflict);

    let unchanged = service
        .prepare_lower_kpi_classification(
            PrepareLowerKpiClassification {
                kpi_id: kpi.id.clone(),
                expected_version: kpi.version,
                proposed_classification: DataClassification::Restricted,
                rationale: rationale("Synthetic rationale."),
                context: context("kpi-unchanged"),
            },
            &mut ids,
        )
        .unwrap_err();
    assert_eq!(unchanged.code(), ErrorCode::DomainConflict);
}

fn observation_id() -> KpiObservationId {
    KpiObservationId::parse("observation-lowering").unwrap()
}

fn create_restricted_observation(
    service: &mut InMemoryPortfolioService<FixedClock, SequentialAuditIds>,
) -> KpiObservationRecord {
    let kpi = create_restricted_kpi(service);
    service
        .create_kpi_observation(CreateKpiObservation {
            id: observation_id(),
            kpi_id: kpi.id.clone(),
            value: short("42"),
            observed_at: UtcTimestamp::from_unix_millis(500),
            source: long("Synthetic evidence record."),
            classification: Some(DataClassification::Restricted),
            provenance: provenance(),
            context: context("create-observation"),
        })
        .unwrap()
        .record
}

#[test]
fn kpi_observation_prepare_then_approve_atomically_lowers_classification() {
    let mut service = service();
    let mut ids = SequentialIds(0);
    let observation = create_restricted_observation(&mut service);

    let prepared = service
        .prepare_lower_kpi_observation_classification(
            PrepareLowerKpiObservationClassification {
                observation_id: observation.id.clone(),
                expected_version: observation.version,
                proposed_classification: DataClassification::Internal,
                rationale: rationale("Synthetic rationale."),
                context: context("observation-lower-prepare"),
            },
            &mut ids,
        )
        .unwrap_or_else(|error| panic!("prepare: {error}"));

    let outcome = service
        .approve_and_execute_lower_kpi_observation_classification(
            ApproveAndExecuteLowerKpiObservationClassification {
                approval: approval(&prepared, "observation-lower-approve"),
                context: context("observation-lower-approve"),
            },
            &mut ids,
            &AllowHeadOfProducts,
        )
        .unwrap_or_else(|error| panic!("approve: {error}"));

    assert_eq!(outcome.record.classification, DataClassification::Internal);
    assert_eq!(outcome.record.version, observation.version.next().unwrap());
    assert!(outcome.record.updated_at >= observation.updated_at);
    assert!(matches!(
        service.audit_events().last().map(|event| event.target()),
        Some(AuditTarget::KpiObservation(id)) if id == &observation_id()
    ));
}

#[test]
fn kpi_observation_prepare_rejects_a_stale_version_and_a_non_lowering_proposal() {
    let mut service = service();
    let mut ids = SequentialIds(0);
    let observation = create_restricted_observation(&mut service);

    let stale = service
        .prepare_lower_kpi_observation_classification(
            PrepareLowerKpiObservationClassification {
                observation_id: observation.id.clone(),
                expected_version: AggregateVersion::initial().next().unwrap(),
                proposed_classification: DataClassification::Internal,
                rationale: rationale("Synthetic rationale."),
                context: context("observation-stale"),
            },
            &mut ids,
        )
        .unwrap_err();
    assert_eq!(stale.code(), ErrorCode::DomainConflict);

    let unchanged = service
        .prepare_lower_kpi_observation_classification(
            PrepareLowerKpiObservationClassification {
                observation_id: observation.id.clone(),
                expected_version: observation.version,
                proposed_classification: DataClassification::Restricted,
                rationale: rationale("Synthetic rationale."),
                context: context("observation-unchanged"),
            },
            &mut ids,
        )
        .unwrap_err();
    assert_eq!(unchanged.code(), ErrorCode::DomainConflict);
}
