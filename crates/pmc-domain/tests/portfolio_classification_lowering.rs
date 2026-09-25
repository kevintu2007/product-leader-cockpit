//! H2a "Lower Data Classification" for the Portfolio record type -- the
//! first of the nine classified aggregate families to gain the
//! operation named in the frozen DG0 table (`PrepareLowerDataClassification`
//! -> `ApproveAndExecuteLowerDataClassification`).

use std::cell::Cell;
use std::rc::Rc;

use pmc_domain::audit::{AuditActor, AuditEventIdSource, AuditTarget};
use pmc_domain::classification::DataClassification;
use pmc_domain::error::ErrorCode;
use pmc_domain::identity::{
    AggregateVersion, ApprovalReceiptId, AuditEventId, CorrelationId, IdempotencyId, PortfolioId,
    PreparedIntentId,
};
use pmc_domain::portfolio::*;
use pmc_domain::provenance::{Provenance, ProvenanceReference};
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::work_management::{
    ApprovalAuthorizationPort, ApprovalConfirmation, WorkManagementApproval,
};
use pmc_domain::DomainValueError;

#[derive(Clone)]
struct TestClock(Rc<Cell<i64>>);

impl Clock for TestClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(self.0.get())
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

#[derive(Clone, Copy)]
struct DenyEveryone;
impl ApprovalAuthorizationPort for DenyEveryone {
    fn authorize(&self, _actor: AuditActor) -> bool {
        false
    }
}

fn clock() -> (TestClock, Rc<Cell<i64>>) {
    let now = Rc::new(Cell::new(1_000));
    (TestClock(now.clone()), now)
}

fn service() -> InMemoryPortfolioService<TestClock, SequentialAuditIds> {
    InMemoryPortfolioService::new(clock().0, SequentialAuditIds(0))
}

fn short(value: &str) -> ShortText {
    ShortText::parse(value).unwrap_or_else(|error| panic!("synthetic short text: {error}"))
}

fn long(value: &str) -> LongText {
    LongText::parse(value).unwrap_or_else(|error| panic!("synthetic long text: {error}"))
}

fn rationale(value: &str) -> pmc_domain::work_management::WorkManagementRationale {
    pmc_domain::work_management::WorkManagementRationale::parse(value)
        .unwrap_or_else(|error| panic!("synthetic rationale: {error}"))
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
        ProvenanceReference::parse("synthetic-portfolio-lowering-v1")
            .unwrap_or_else(|error| panic!("synthetic provenance: {error}")),
    )
}

fn portfolio_id() -> PortfolioId {
    PortfolioId::parse("portfolio-lowering")
        .unwrap_or_else(|error| panic!("synthetic portfolio id: {error}"))
}

fn create_restricted_portfolio(
    service: &mut InMemoryPortfolioService<TestClock, SequentialAuditIds>,
) -> PortfolioRecord {
    service
        .create_portfolio(CreatePortfolio {
            id: portfolio_id(),
            name: short("Synthetic Product Portfolio"),
            details: long("Public-safe portfolio used only by deterministic tests."),
            classification: Some(DataClassification::Restricted),
            provenance: provenance(),
            context: context("create-portfolio"),
        })
        .unwrap_or_else(|error| panic!("create portfolio: {error}"))
        .record
}

fn approval(
    prepared: &pmc_domain::work_management::WorkManagementPreparedIntent,
    idempotency_key: &str,
) -> WorkManagementApproval {
    WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse(idempotency_key).unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap_or_else(|error| panic!("synthetic approval: {error}"))
}

#[test]
fn prepare_then_approve_atomically_lowers_classification_and_bumps_version() {
    let mut service = service();
    let mut ids = SequentialIds(0);
    let portfolio = create_restricted_portfolio(&mut service);

    let prepared = service
        .prepare_lower_portfolio_classification(
            PrepareLowerPortfolioClassification {
                portfolio_id: portfolio.id.clone(),
                expected_version: portfolio.version,
                proposed_classification: DataClassification::Internal,
                rationale: rationale("Synthetic downstream impact review completed."),
                context: context("lower-prepare"),
            },
            &mut ids,
        )
        .unwrap_or_else(|error| panic!("prepare: {error}"));

    let outcome = service
        .approve_and_execute_lower_portfolio_classification(
            ApproveAndExecuteLowerPortfolioClassification {
                approval: approval(&prepared, "lower-approve"),
                context: context("lower-approve"),
            },
            &mut ids,
            &AllowHeadOfProducts,
        )
        .unwrap_or_else(|error| panic!("approve: {error}"));

    assert_eq!(outcome.record.classification, DataClassification::Internal);
    assert_eq!(outcome.record.version, portfolio.version.next().unwrap());
    assert_eq!(
        service.inspect_portfolio(&portfolio_id()),
        Some(outcome.record.clone())
    );
    assert!(matches!(
        service.audit_events().last().map(|event| event.target()),
        Some(AuditTarget::Portfolio(id)) if id == &portfolio_id()
    ));
}

#[test]
fn prepare_rejects_a_stale_expected_version() {
    let mut service = service();
    let mut ids = SequentialIds(0);
    let portfolio = create_restricted_portfolio(&mut service);

    let error = service
        .prepare_lower_portfolio_classification(
            PrepareLowerPortfolioClassification {
                portfolio_id: portfolio.id.clone(),
                expected_version: AggregateVersion::initial().next().unwrap(),
                proposed_classification: DataClassification::Internal,
                rationale: rationale("Synthetic rationale."),
                context: context("lower-stale"),
            },
            &mut ids,
        )
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::DomainConflict);
}

#[test]
fn prepare_rejects_an_unknown_portfolio() {
    let mut service = service();
    let mut ids = SequentialIds(0);

    let error = service
        .prepare_lower_portfolio_classification(
            PrepareLowerPortfolioClassification {
                portfolio_id: portfolio_id(),
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Internal,
                rationale: rationale("Synthetic rationale."),
                context: context("lower-missing"),
            },
            &mut ids,
        )
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::DomainNotFound);
}

#[test]
fn prepare_rejects_a_proposal_that_is_not_a_genuine_lowering() {
    let mut service = service();
    let mut ids = SequentialIds(0);
    let portfolio = create_restricted_portfolio(&mut service);

    // Same classification: not a lowering.
    let unchanged = service
        .prepare_lower_portfolio_classification(
            PrepareLowerPortfolioClassification {
                portfolio_id: portfolio.id.clone(),
                expected_version: portfolio.version,
                proposed_classification: DataClassification::Restricted,
                rationale: rationale("Synthetic rationale."),
                context: context("lower-unchanged"),
            },
            &mut ids,
        )
        .unwrap_err();
    assert_eq!(unchanged.code(), ErrorCode::DomainConflict);

    // Unclassified is the fail-closed default, not a real, chosen
    // classification -- it can never be a genuine lowering target either.
    let raised = service
        .prepare_lower_portfolio_classification(
            PrepareLowerPortfolioClassification {
                portfolio_id: portfolio.id.clone(),
                expected_version: portfolio.version,
                proposed_classification: DataClassification::Unclassified,
                rationale: rationale("Synthetic rationale."),
                context: context("lower-raise-attempt"),
            },
            &mut ids,
        )
        .unwrap_err();
    // Unclassified is the fail-closed default, never a real lowering target.
    assert_eq!(raised.code(), ErrorCode::DomainConflict);
}

#[test]
fn prepare_is_idempotent_on_the_same_idempotency_id() {
    let mut service = service();
    let mut ids = SequentialIds(0);
    let portfolio = create_restricted_portfolio(&mut service);

    let intent = PrepareLowerPortfolioClassification {
        portfolio_id: portfolio.id.clone(),
        expected_version: portfolio.version,
        proposed_classification: DataClassification::Internal,
        rationale: rationale("Synthetic rationale."),
        context: context("lower-idem"),
    };
    let first = service
        .prepare_lower_portfolio_classification(intent.clone(), &mut ids)
        .unwrap_or_else(|error| panic!("first prepare: {error}"));
    let replay = service
        .prepare_lower_portfolio_classification(intent, &mut ids)
        .unwrap_or_else(|error| panic!("replay prepare: {error}"));
    assert_eq!(first.id(), replay.id());
    assert_eq!(first.payload_digest(), replay.payload_digest());

    let conflicting = service
        .prepare_lower_portfolio_classification(
            PrepareLowerPortfolioClassification {
                portfolio_id: portfolio.id.clone(),
                expected_version: portfolio.version,
                proposed_classification: DataClassification::Public,
                rationale: rationale("A different rationale."),
                context: context("lower-idem"),
            },
            &mut ids,
        )
        .unwrap_err();
    assert_eq!(conflicting.code(), ErrorCode::DomainIdempotencyConflict);
}

#[test]
fn approve_rejects_when_the_authorization_port_denies_the_actor() {
    let mut service = service();
    let mut ids = SequentialIds(0);
    let portfolio = create_restricted_portfolio(&mut service);

    let prepared = service
        .prepare_lower_portfolio_classification(
            PrepareLowerPortfolioClassification {
                portfolio_id: portfolio.id.clone(),
                expected_version: portfolio.version,
                proposed_classification: DataClassification::Internal,
                rationale: rationale("Synthetic rationale."),
                context: context("lower-deny-prepare"),
            },
            &mut ids,
        )
        .unwrap_or_else(|error| panic!("prepare: {error}"));

    let error = service
        .approve_and_execute_lower_portfolio_classification(
            ApproveAndExecuteLowerPortfolioClassification {
                approval: approval(&prepared, "lower-deny"),
                context: context("lower-deny"),
            },
            &mut ids,
            &DenyEveryone,
        )
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::SecurityPolicyDenied);
    assert_eq!(service.inspect_portfolio(&portfolio_id()), Some(portfolio));
}

#[test]
fn approve_rejects_a_mismatched_acknowledged_digest_without_effect() {
    let mut service = service();
    let mut ids = SequentialIds(0);
    let portfolio = create_restricted_portfolio(&mut service);

    let prepared = service
        .prepare_lower_portfolio_classification(
            PrepareLowerPortfolioClassification {
                portfolio_id: portfolio.id.clone(),
                expected_version: portfolio.version,
                proposed_classification: DataClassification::Internal,
                rationale: rationale("Synthetic rationale."),
                context: context("lower-digest-prepare"),
            },
            &mut ids,
        )
        .unwrap_or_else(|error| panic!("prepare: {error}"));

    let wrong_digest_approval = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        pmc_domain::work_management::WorkManagementPayloadDigest::from_persisted("b".repeat(64))
            .unwrap(),
        IdempotencyId::parse("lower-digest").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();

    let error = service
        .approve_and_execute_lower_portfolio_classification(
            ApproveAndExecuteLowerPortfolioClassification {
                approval: wrong_digest_approval,
                context: context("lower-digest"),
            },
            &mut ids,
            &AllowHeadOfProducts,
        )
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
    assert_eq!(service.inspect_portfolio(&portfolio_id()), Some(portfolio));
}

#[test]
fn approve_rejects_an_expired_prepared_intent_without_effect() {
    let (test_clock, now) = clock();
    let mut service = InMemoryPortfolioService::new(test_clock, SequentialAuditIds(0));
    let mut ids = SequentialIds(0);
    let portfolio = create_restricted_portfolio(&mut service);

    let prepared = service
        .prepare_lower_portfolio_classification(
            PrepareLowerPortfolioClassification {
                portfolio_id: portfolio.id.clone(),
                expected_version: portfolio.version,
                proposed_classification: DataClassification::Internal,
                rationale: rationale("Synthetic rationale."),
                context: context("lower-expiry-prepare"),
            },
            &mut ids,
        )
        .unwrap_or_else(|error| panic!("prepare: {error}"));

    now.set(prepared.preview().expires_at().unix_millis());

    let error = service
        .approve_and_execute_lower_portfolio_classification(
            ApproveAndExecuteLowerPortfolioClassification {
                approval: approval(&prepared, "lower-expired"),
                context: context("lower-expired"),
            },
            &mut ids,
            &AllowHeadOfProducts,
        )
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
    assert_eq!(service.inspect_portfolio(&portfolio_id()), Some(portfolio));
}

#[test]
fn approve_rejects_when_the_portfolio_drifted_since_prepare() {
    let mut service = service();
    let mut ids = SequentialIds(0);
    let portfolio = create_restricted_portfolio(&mut service);

    let prepared = service
        .prepare_lower_portfolio_classification(
            PrepareLowerPortfolioClassification {
                portfolio_id: portfolio.id.clone(),
                expected_version: portfolio.version,
                proposed_classification: DataClassification::Internal,
                rationale: rationale("Synthetic rationale."),
                context: context("lower-drift-prepare"),
            },
            &mut ids,
        )
        .unwrap_or_else(|error| panic!("prepare: {error}"));

    // An unrelated ordinary update bumps the version out from under the
    // prepared intent.
    let drifted = service
        .update_portfolio_details(UpdatePortfolioDetails {
            id: portfolio.id.clone(),
            expected_version: portfolio.version,
            name: short("Renamed Portfolio"),
            details: long("Renamed by an unrelated concurrent operator."),
            classification: None,
            context: context("lower-drift-update"),
        })
        .unwrap_or_else(|error| panic!("drift update: {error}"))
        .record;

    let error = service
        .approve_and_execute_lower_portfolio_classification(
            ApproveAndExecuteLowerPortfolioClassification {
                approval: approval(&prepared, "lower-drift-approve"),
                context: context("lower-drift-approve"),
            },
            &mut ids,
            &AllowHeadOfProducts,
        )
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
    assert_eq!(service.inspect_portfolio(&portfolio_id()), Some(drifted));
}

#[test]
fn approve_replays_the_original_outcome_without_a_second_effect() {
    let mut service = service();
    let mut ids = SequentialIds(0);
    let portfolio = create_restricted_portfolio(&mut service);

    let prepared = service
        .prepare_lower_portfolio_classification(
            PrepareLowerPortfolioClassification {
                portfolio_id: portfolio.id.clone(),
                expected_version: portfolio.version,
                proposed_classification: DataClassification::Internal,
                rationale: rationale("Synthetic rationale."),
                context: context("lower-replay-prepare"),
            },
            &mut ids,
        )
        .unwrap_or_else(|error| panic!("prepare: {error}"));

    let command = ApproveAndExecuteLowerPortfolioClassification {
        approval: approval(&prepared, "lower-replay"),
        context: context("lower-replay"),
    };
    let first = service
        .approve_and_execute_lower_portfolio_classification(
            command.clone(),
            &mut ids,
            &AllowHeadOfProducts,
        )
        .unwrap_or_else(|error| panic!("first approve: {error}"));
    let audit_count = service.audit_events().len();
    let replay = service
        .approve_and_execute_lower_portfolio_classification(command, &mut ids, &AllowHeadOfProducts)
        .unwrap_or_else(|error| panic!("replay approve: {error}"));
    assert_eq!(replay, first);
    assert_eq!(service.audit_events().len(), audit_count);
}
