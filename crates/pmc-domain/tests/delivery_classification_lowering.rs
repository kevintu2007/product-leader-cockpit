//! H2a "Lower Data Classification" for the Delivery family
//! (Initiative, Project, Milestone) -- the first of the two remaining
//! families beyond the original six (Portfolio family, Action, Decision,
//! Risk, Issue). `InMemoryDeliveryService<C, I>` was never built with H2a
//! Prepare/Approve machinery (only `Clock` + `AuditEventIdSource`), exactly
//! like Portfolio's original shape -- so this mirrors Portfolio's lowering
//! design precisely, including the same method-level-generic ID-source
//! workaround rather than a breaking constructor change, and the same
//! shared `prepared_lowerings`/`prepared_lowering_requests` fields reused
//! across all three record types.

use pmc_domain::audit::{AuditActor, AuditEventIdSource};
use pmc_domain::classification::DataClassification;
use pmc_domain::delivery::*;
use pmc_domain::error::ErrorCode;
use pmc_domain::identity::{
    AggregateVersion, ApprovalReceiptId, AuditEventId, CorrelationId, IdempotencyId, InitiativeId,
    MilestoneId, PreparedIntentId, ProjectId,
};
use pmc_domain::provenance::{Provenance, ProvenanceReference};
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::work_management::{
    ApprovalAuthorizationPort, ApprovalConfirmation, WorkManagementApproval,
    WorkManagementPreparedIntent, WorkManagementRationale,
};
use pmc_domain::DomainValueError;

#[derive(Clone, Copy)]
struct FixedClock(UtcTimestamp);
impl Clock for FixedClock {
    fn now(&self) -> UtcTimestamp {
        self.0
    }
}

#[derive(Default)]
struct SequentialAuditIds(u64);
impl AuditEventIdSource for SequentialAuditIds {
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        self.0 += 1;
        AuditEventId::parse(format!("delivery-audit-{}", self.0))
    }
}

struct SequentialIds(u64);
impl DeliveryClassificationLoweringIdSource for SequentialIds {
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

fn service() -> InMemoryDeliveryService<FixedClock, SequentialAuditIds> {
    InMemoryDeliveryService::new(
        FixedClock(UtcTimestamp::from_unix_millis(1_000)),
        SequentialAuditIds::default(),
    )
}
fn context(suffix: &str) -> OperationContext {
    OperationContext {
        correlation_id: CorrelationId::parse(format!("correlation-{suffix}")).unwrap(),
        idempotency_id: IdempotencyId::parse(format!("idempotency-{suffix}")).unwrap(),
    }
}
fn name(value: &str) -> RecordName {
    RecordName::parse(value, &CorrelationId::parse("correlation-text").unwrap()).unwrap()
}
fn outcome(value: &str) -> DefinedOutcome {
    DefinedOutcome::parse(value, &CorrelationId::parse("correlation-text").unwrap()).unwrap()
}
fn criteria(value: &str) -> VerificationCriteria {
    VerificationCriteria::parse(value, &CorrelationId::parse("correlation-text").unwrap()).unwrap()
}
fn provenance() -> Provenance {
    Provenance::SyntheticFixture(ProvenanceReference::parse("public-safe-scenario").unwrap())
}
fn rationale(value: &str) -> WorkManagementRationale {
    WorkManagementRationale::parse(value).unwrap()
}
fn approval(prepared: &WorkManagementPreparedIntent, id: &str) -> WorkManagementApproval {
    WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse(format!("idempotency-{id}")).unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap()
}

fn restricted_initiative(
    s: &mut InMemoryDeliveryService<FixedClock, SequentialAuditIds>,
) -> Initiative {
    s.create_initiative(CreateInitiative {
        context: context("i-create"),
        id: InitiativeId::parse("initiative-lowering").unwrap(),
        name: name("Synthetic Initiative"),
        defined_outcome: outcome("Validate synthetic adoption"),
        classification: Some(DataClassification::Restricted),
        provenance: provenance(),
    })
    .unwrap()
}

#[test]
fn initiative_prepare_then_approve_atomically_lowers_classification() {
    let mut s = service();
    let mut ids = SequentialIds(0);
    let initiative = restricted_initiative(&mut s);

    let prepared = s
        .prepare_lower_initiative_classification(
            PrepareLowerInitiativeClassification {
                id: initiative.id().clone(),
                expected_version: initiative.version(),
                proposed_classification: DataClassification::Internal,
                rationale: rationale("Synthetic rationale."),
                context: context("i-lower-prepare"),
            },
            &mut ids,
        )
        .unwrap_or_else(|error| panic!("prepare: {error}"));

    let outcome = s
        .approve_and_execute_lower_initiative_classification(
            ApproveAndExecuteLowerInitiativeClassification {
                approval: approval(&prepared, "i-lower-approve"),
                context: context("i-lower-approve"),
            },
            &mut ids,
            &AllowHeadOfProducts,
        )
        .unwrap_or_else(|error| panic!("approve: {error}"));

    assert_eq!(outcome.classification(), DataClassification::Internal);
    assert_eq!(outcome.version(), initiative.version().next().unwrap());
}

#[test]
fn initiative_prepare_rejects_a_stale_version_and_a_non_lowering_proposal() {
    let mut s = service();
    let mut ids = SequentialIds(0);
    let initiative = restricted_initiative(&mut s);

    let stale = s
        .prepare_lower_initiative_classification(
            PrepareLowerInitiativeClassification {
                id: initiative.id().clone(),
                expected_version: AggregateVersion::initial().next().unwrap(),
                proposed_classification: DataClassification::Internal,
                rationale: rationale("Synthetic rationale."),
                context: context("i-stale"),
            },
            &mut ids,
        )
        .unwrap_err();
    assert_eq!(stale.code(), ErrorCode::DomainConflict);

    let unchanged = s
        .prepare_lower_initiative_classification(
            PrepareLowerInitiativeClassification {
                id: initiative.id().clone(),
                expected_version: initiative.version(),
                proposed_classification: DataClassification::Restricted,
                rationale: rationale("Synthetic rationale."),
                context: context("i-unchanged"),
            },
            &mut ids,
        )
        .unwrap_err();
    assert_eq!(unchanged.code(), ErrorCode::DomainConflict);
}

fn restricted_project(s: &mut InMemoryDeliveryService<FixedClock, SequentialAuditIds>) -> Project {
    s.create_project(CreateProject {
        context: context("p-create"),
        id: ProjectId::parse("project-lowering").unwrap(),
        name: name("Synthetic Project"),
        start_at: UtcTimestamp::from_unix_millis(10),
        end_at: UtcTimestamp::from_unix_millis(20),
        classification: Some(DataClassification::Restricted),
        provenance: provenance(),
    })
    .unwrap()
}

#[test]
fn project_prepare_then_approve_atomically_lowers_classification() {
    let mut s = service();
    let mut ids = SequentialIds(0);
    let project = restricted_project(&mut s);

    let prepared = s
        .prepare_lower_project_classification(
            PrepareLowerProjectClassification {
                id: project.id().clone(),
                expected_version: project.version(),
                proposed_classification: DataClassification::Internal,
                rationale: rationale("Synthetic rationale."),
                context: context("p-lower-prepare"),
            },
            &mut ids,
        )
        .unwrap_or_else(|error| panic!("prepare: {error}"));

    let outcome = s
        .approve_and_execute_lower_project_classification(
            ApproveAndExecuteLowerProjectClassification {
                approval: approval(&prepared, "p-lower-approve"),
                context: context("p-lower-approve"),
            },
            &mut ids,
            &AllowHeadOfProducts,
        )
        .unwrap_or_else(|error| panic!("approve: {error}"));

    assert_eq!(outcome.classification(), DataClassification::Internal);
    assert_eq!(outcome.version(), project.version().next().unwrap());
}

#[test]
fn project_prepare_rejects_a_stale_version_and_a_non_lowering_proposal() {
    let mut s = service();
    let mut ids = SequentialIds(0);
    let project = restricted_project(&mut s);

    let stale = s
        .prepare_lower_project_classification(
            PrepareLowerProjectClassification {
                id: project.id().clone(),
                expected_version: AggregateVersion::initial().next().unwrap(),
                proposed_classification: DataClassification::Internal,
                rationale: rationale("Synthetic rationale."),
                context: context("p-stale"),
            },
            &mut ids,
        )
        .unwrap_err();
    assert_eq!(stale.code(), ErrorCode::DomainConflict);

    let unchanged = s
        .prepare_lower_project_classification(
            PrepareLowerProjectClassification {
                id: project.id().clone(),
                expected_version: project.version(),
                proposed_classification: DataClassification::Restricted,
                rationale: rationale("Synthetic rationale."),
                context: context("p-unchanged"),
            },
            &mut ids,
        )
        .unwrap_err();
    assert_eq!(unchanged.code(), ErrorCode::DomainConflict);
}

fn restricted_milestone(
    s: &mut InMemoryDeliveryService<FixedClock, SequentialAuditIds>,
) -> Milestone {
    s.create_project(CreateProject {
        context: context("m-project"),
        id: ProjectId::parse("project-for-milestone").unwrap(),
        name: name("Synthetic Project"),
        start_at: UtcTimestamp::from_unix_millis(1_000),
        end_at: UtcTimestamp::from_unix_millis(5_000),
        classification: Some(DataClassification::Restricted),
        provenance: provenance(),
    })
    .unwrap();
    s.create_milestone(CreateMilestone {
        context: context("m-create"),
        id: MilestoneId::parse("milestone-lowering").unwrap(),
        project_id: ProjectId::parse("project-for-milestone").unwrap(),
        name: name("Synthetic Milestone"),
        verification_criteria: criteria("Signed synthetic checklist attached"),
        due_at: UtcTimestamp::from_unix_millis(4_000),
        classification: Some(DataClassification::Restricted),
        provenance: provenance(),
    })
    .unwrap()
}

#[test]
fn milestone_prepare_then_approve_atomically_lowers_classification() {
    let mut s = service();
    let mut ids = SequentialIds(0);
    let milestone = restricted_milestone(&mut s);

    let prepared = s
        .prepare_lower_milestone_classification(
            PrepareLowerMilestoneClassification {
                id: milestone.id().clone(),
                expected_version: milestone.version(),
                proposed_classification: DataClassification::Internal,
                rationale: rationale("Synthetic rationale."),
                context: context("m-lower-prepare"),
            },
            &mut ids,
        )
        .unwrap_or_else(|error| panic!("prepare: {error}"));

    let outcome = s
        .approve_and_execute_lower_milestone_classification(
            ApproveAndExecuteLowerMilestoneClassification {
                approval: approval(&prepared, "m-lower-approve"),
                context: context("m-lower-approve"),
            },
            &mut ids,
            &AllowHeadOfProducts,
        )
        .unwrap_or_else(|error| panic!("approve: {error}"));

    assert_eq!(outcome.classification(), DataClassification::Internal);
    assert_eq!(outcome.version(), milestone.version().next().unwrap());
}

#[test]
fn milestone_prepare_rejects_a_stale_version_and_a_non_lowering_proposal() {
    let mut s = service();
    let mut ids = SequentialIds(0);
    let milestone = restricted_milestone(&mut s);

    let stale = s
        .prepare_lower_milestone_classification(
            PrepareLowerMilestoneClassification {
                id: milestone.id().clone(),
                expected_version: AggregateVersion::initial().next().unwrap(),
                proposed_classification: DataClassification::Internal,
                rationale: rationale("Synthetic rationale."),
                context: context("m-stale"),
            },
            &mut ids,
        )
        .unwrap_err();
    assert_eq!(stale.code(), ErrorCode::DomainConflict);

    let unchanged = s
        .prepare_lower_milestone_classification(
            PrepareLowerMilestoneClassification {
                id: milestone.id().clone(),
                expected_version: milestone.version(),
                proposed_classification: DataClassification::Restricted,
                rationale: rationale("Synthetic rationale."),
                context: context("m-unchanged"),
            },
            &mut ids,
        )
        .unwrap_err();
    assert_eq!(unchanged.code(), ErrorCode::DomainConflict);
}
