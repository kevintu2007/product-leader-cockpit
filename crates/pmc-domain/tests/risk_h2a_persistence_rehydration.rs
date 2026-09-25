//! RED contract for Risk H2a terminal-denial persistence.
//!
//! This intentionally uses the currently available typed Risk aggregate
//! snapshot as the restart boundary.  The test is expected to turn green when
//! that boundary also carries the prepared intent and terminal replay record.

use std::cell::Cell;
use std::rc::Rc;

use pmc_domain::audit::{
    AuditAction, AuditActor, AuditApprovalOutcome, AuditDisposition, AuditEffectScope, AuditEvent,
    AuditEventCode, AuditExecutionOutcome, AuditModule, AuditPolicyOutcome, AuditTarget,
};
use pmc_domain::classification::DataClassification;
use pmc_domain::error::{DomainError, ErrorCode, MessageKey};
use pmc_domain::identity::{
    ApprovalReceiptId, AuditEventId, CorrelationId, IdempotencyId, IssueId, PreparedIntentId,
    RiskId,
};
use pmc_domain::risks::{
    AllowRiskEvidence, ApproveAndExecuteCloseRisk, CreateRisk, InMemoryRiskService,
    PrepareCloseRisk, PrepareRecordRiskOccurrence, RecordedRiskClassification, RiskExecutionPolicy,
    RiskExecutionPolicyPort, RiskH2aPersistenceDecodeInput, RiskH2aTerminalOperation,
    RiskH2aTerminalReplay, RiskOperationContext, RiskPersistenceSnapshot, RiskServiceIdSource,
};
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::work_management::{
    ApprovalAuthorizationPort, ApprovalConfirmation, WorkManagementApproval,
    WorkManagementOperation, WorkManagementPreparedIntent,
};

#[derive(Clone)]
struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(100)
    }
}

#[derive(Clone)]
struct TestIds(Rc<Cell<u64>>);

impl RiskServiceIdSource for TestIds {
    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<PreparedIntentId, pmc_domain::DomainValueError> {
        let id = self.0.get() + 1;
        self.0.set(id);
        PreparedIntentId::parse(format!("prepared-risk-h2a-{id}"))
    }

    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<ApprovalReceiptId, pmc_domain::DomainValueError> {
        let id = self.0.get() + 1;
        self.0.set(id);
        ApprovalReceiptId::parse(format!("receipt-risk-h2a-{id}"))
    }

    fn next_audit_event_id(&mut self) -> Result<AuditEventId, pmc_domain::DomainValueError> {
        let id = self.0.get() + 1;
        self.0.set(id);
        AuditEventId::parse(format!("audit-risk-h2a-{id}"))
    }
}

#[derive(Clone)]
struct MutablePolicy(Rc<Cell<bool>>);

impl RiskExecutionPolicyPort for MutablePolicy {
    fn current_policy(&self, _: &WorkManagementOperation) -> RiskExecutionPolicy {
        if self.0.get() {
            RiskExecutionPolicy::Allowed
        } else {
            RiskExecutionPolicy::Denied
        }
    }
}

#[derive(Clone, Copy)]
struct AllowApproval;

impl ApprovalAuthorizationPort for AllowApproval {
    fn authorize(&self, _: AuditActor) -> bool {
        true
    }
}

fn text<const N: usize>(value: &str) -> pmc_domain::BoundedText<N> {
    pmc_domain::BoundedText::parse(value.to_owned()).unwrap()
}

fn context(id: &str, correlation: &str) -> RiskOperationContext {
    RiskOperationContext {
        idempotency_id: IdempotencyId::parse(id).unwrap(),
        correlation_id: CorrelationId::parse(correlation).unwrap(),
    }
}

fn service(
    policy: Rc<Cell<bool>>,
    ids: Rc<Cell<u64>>,
) -> InMemoryRiskService<
    FixedClock,
    TestIds,
    AllowApproval,
    MutablePolicy,
    AllowRiskEvidence,
    RecordedRiskClassification,
> {
    InMemoryRiskService::new(
        FixedClock,
        TestIds(ids),
        AllowApproval,
        MutablePolicy(policy),
        AllowRiskEvidence,
        RecordedRiskClassification,
    )
}

#[test]
fn post_start_denial_survives_typed_restart_and_replays_without_audit_or_effect() {
    let policy = Rc::new(Cell::new(true));
    let ids = Rc::new(Cell::new(0));
    let mut original = service(policy.clone(), ids.clone());

    let created = original
        .create_risk(CreateRisk {
            id: RiskId::parse("risk-h2a-rehydrate").unwrap(),
            title: text("Synthetic restart denial"),
            details: text("Synthetic restart denial details"),
            classification: DataClassification::Internal,
            context: context("risk-h2a-create", "risk-h2a-create-correlation"),
        })
        .unwrap();
    let prepared = original
        .prepare_record_risk_occurrence(PrepareRecordRiskOccurrence {
            risk_id: created.record.id().clone(),
            expected_version: created.record.version(),
            issue_id: IssueId::parse("issue-h2a-rehydrate").unwrap(),
            context: context("risk-h2a-prepare", "risk-h2a-prepare-correlation"),
        })
        .unwrap();
    let approval = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("risk-h2a-execute").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();

    policy.set(false);
    let original_error = original
        .approve_and_execute_record_risk_occurrence(
            pmc_domain::risks::ApproveAndExecuteRecordRiskOccurrence {
                approval: approval.clone(),
                context: context("risk-h2a-execute", "risk-h2a-original-correlation"),
            },
        )
        .unwrap_err();
    assert_eq!(original_error.code(), ErrorCode::SecurityPolicyDenied);

    let restored_risk = original.risk(created.record.id()).unwrap().clone();
    let persisted = original.persistence_snapshot_with_h2a().unwrap();

    // Re-open a fresh service only from the typed Risk H2a boundary.
    policy.set(true);
    let mut reopened = InMemoryRiskService::rehydrate_with_h2a(
        FixedClock,
        TestIds(ids),
        AllowApproval,
        MutablePolicy(policy),
        AllowRiskEvidence,
        RecordedRiskClassification,
        persisted,
    )
    .unwrap();
    assert_eq!(reopened.risk(restored_risk.id()).unwrap(), &restored_risk);

    let audits_before_replay = reopened.audit_events().len();
    let replayed_error = reopened
        .approve_and_execute_record_risk_occurrence(
            pmc_domain::risks::ApproveAndExecuteRecordRiskOccurrence {
                approval,
                context: context("risk-h2a-execute", "risk-h2a-replay-correlation"),
            },
        )
        .unwrap_err();

    assert_eq!(replayed_error, original_error);
    assert_eq!(reopened.audit_events().len(), audits_before_replay);
    assert_eq!(reopened.risk(restored_risk.id()).unwrap(), &restored_risk);
    assert!(reopened
        .issue(&IssueId::parse("issue-h2a-rehydrate").unwrap())
        .is_none());
}

#[test]
fn typed_terminal_decode_rehydrates_policy_denial_without_generic_state_import() {
    let policy = Rc::new(Cell::new(true));
    let ids = Rc::new(Cell::new(0));
    let mut original = service(policy.clone(), ids.clone());

    let created = original
        .create_risk(CreateRisk {
            id: RiskId::parse("risk-h2a-decoder").unwrap(),
            title: text("Synthetic typed decoder denial"),
            details: text("Synthetic typed decoder denial details"),
            classification: DataClassification::Internal,
            context: context(
                "risk-h2a-decoder-create",
                "risk-h2a-decoder-create-correlation",
            ),
        })
        .unwrap();
    let prepared = original
        .prepare_record_risk_occurrence(PrepareRecordRiskOccurrence {
            risk_id: created.record.id().clone(),
            expected_version: created.record.version(),
            issue_id: IssueId::parse("issue-h2a-decoder").unwrap(),
            context: context(
                "risk-h2a-decoder-prepare",
                "risk-h2a-decoder-prepare-correlation",
            ),
        })
        .unwrap();
    let approval = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("risk-h2a-decoder-execute").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();

    policy.set(false);
    let original_error = original
        .approve_and_execute_record_risk_occurrence(
            pmc_domain::risks::ApproveAndExecuteRecordRiskOccurrence {
                approval: approval.clone(),
                context: context("risk-h2a-decoder-execute", "risk-h2a-decoder-correlation"),
            },
        )
        .unwrap_err();
    let audit = original.audit_events().last().unwrap().clone();

    let base = RiskPersistenceSnapshot::try_new(
        vec![original.risk(created.record.id()).unwrap().clone()],
        vec![],
        vec![],
    )
    .unwrap();
    let duplicate_context = context(
        "risk-h2a-decoder-execute-duplicate",
        "risk-h2a-decoder-duplicate-correlation",
    );
    let duplicate_approval = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        duplicate_context.idempotency_id.clone(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    let duplicate_error = DomainError::new(
        ErrorCode::SecurityPolicyDenied,
        MessageKey::parse("risk.security_denied").unwrap(),
        duplicate_context.correlation_id.clone(),
        false,
    );
    let duplicate_audit = AuditEvent::new(
        AuditEventId::parse("audit-risk-h2a-duplicate").unwrap(),
        UtcTimestamp::from_unix_millis(100),
        AuditActor::HeadOfProducts,
        AuditAction::new(
            AuditModule::WorkManagement,
            AuditEventCode::parse("risk.occurrence_denied").unwrap(),
            AuditTarget::Risk(created.record.id().clone()),
        ),
        duplicate_context.correlation_id.clone(),
        AuditDisposition::new(
            AuditPolicyOutcome::Denied,
            AuditApprovalOutcome::NotRequired,
            AuditExecutionOutcome::NotAttempted,
            AuditEffectScope::None,
            vec![],
        )
        .unwrap(),
    );
    assert!(RiskH2aPersistenceDecodeInput::new(
        base.clone(),
        vec![
            RiskH2aTerminalReplay::new(
                RiskH2aTerminalOperation::RecordOccurrence,
                created.record.id().clone(),
                prepared.clone(),
                approval.clone(),
                context("risk-h2a-decoder-execute", "risk-h2a-decoder-correlation"),
                original_error.clone(),
                audit.clone(),
            ),
            RiskH2aTerminalReplay::new(
                RiskH2aTerminalOperation::RecordOccurrence,
                created.record.id().clone(),
                prepared.clone(),
                duplicate_approval,
                duplicate_context,
                duplicate_error,
                duplicate_audit,
            ),
        ],
    )
    .decode()
    .is_err());
    let stale_prepared = WorkManagementPreparedIntent::prepare(
        prepared.id().clone(),
        WorkManagementOperation::RecordRiskOccurrence {
            risk_id: created.record.id().clone(),
            risk_version: pmc_domain::identity::AggregateVersion::new(2).unwrap(),
            issue_id: IssueId::parse("issue-h2a-decoder").unwrap(),
            issue_classification: DataClassification::Internal,
        },
        DataClassification::Internal,
        None,
        UtcTimestamp::from_unix_millis(100),
    )
    .unwrap();
    let stale_approval = WorkManagementApproval::new(
        stale_prepared.id().clone(),
        AuditActor::HeadOfProducts,
        stale_prepared.payload_digest().clone(),
        IdempotencyId::parse("risk-h2a-decoder-execute").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    assert!(RiskH2aPersistenceDecodeInput::new(
        base.clone(),
        vec![RiskH2aTerminalReplay::new(
            RiskH2aTerminalOperation::RecordOccurrence,
            created.record.id().clone(),
            stale_prepared,
            stale_approval,
            context("risk-h2a-decoder-execute", "risk-h2a-decoder-correlation"),
            original_error.clone(),
            audit.clone(),
        )],
    )
    .decode()
    .is_err());
    assert!(RiskH2aPersistenceDecodeInput::new(
        base.clone(),
        vec![RiskH2aTerminalReplay::new(
            RiskH2aTerminalOperation::Close,
            created.record.id().clone(),
            prepared.clone(),
            approval.clone(),
            context("risk-h2a-decoder-execute", "risk-h2a-decoder-correlation"),
            original_error.clone(),
            audit.clone(),
        )],
    )
    .decode()
    .is_err());
    let decoded = RiskH2aPersistenceDecodeInput::new(
        base,
        vec![RiskH2aTerminalReplay::new(
            RiskH2aTerminalOperation::RecordOccurrence,
            created.record.id().clone(),
            prepared,
            approval.clone(),
            context("risk-h2a-decoder-execute", "risk-h2a-decoder-correlation"),
            original_error.clone(),
            audit,
        )],
    )
    .decode()
    .unwrap();

    policy.set(true);
    let mut reopened = InMemoryRiskService::rehydrate_with_h2a(
        FixedClock,
        TestIds(ids),
        AllowApproval,
        MutablePolicy(policy),
        AllowRiskEvidence,
        RecordedRiskClassification,
        decoded,
    )
    .unwrap();
    let audits_before_replay = reopened.audit_events().len();
    let replayed_error = reopened
        .approve_and_execute_record_risk_occurrence(
            pmc_domain::risks::ApproveAndExecuteRecordRiskOccurrence {
                approval,
                context: context("risk-h2a-decoder-execute", "risk-h2a-replay-correlation"),
            },
        )
        .unwrap_err();

    assert_eq!(replayed_error, original_error);
    assert_eq!(reopened.audit_events().len(), audits_before_replay);
}

#[test]
fn typed_terminal_decode_requires_current_close_risk_preview() {
    let policy = Rc::new(Cell::new(true));
    let ids = Rc::new(Cell::new(0));
    let mut original = service(policy.clone(), ids.clone());
    let created = original
        .create_risk(CreateRisk {
            id: RiskId::parse("risk-h2a-close-decoder").unwrap(),
            title: text("Synthetic close decoder denial"),
            details: text("Synthetic close decoder denial details"),
            classification: DataClassification::Internal,
            context: context("risk-h2a-close-create", "risk-h2a-close-create-correlation"),
        })
        .unwrap();
    let prepared = original
        .prepare_close_risk(PrepareCloseRisk {
            risk_id: created.record.id().clone(),
            expected_version: created.record.version(),
            rationale: text("Synthetic close rationale"),
            context: context(
                "risk-h2a-close-prepare",
                "risk-h2a-close-prepare-correlation",
            ),
        })
        .unwrap();
    let approval = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("risk-h2a-close-execute").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();

    policy.set(false);
    let original_error = original
        .approve_and_execute_close_risk(ApproveAndExecuteCloseRisk {
            approval: approval.clone(),
            context: context("risk-h2a-close-execute", "risk-h2a-close-correlation"),
        })
        .unwrap_err();
    let decoded = RiskH2aPersistenceDecodeInput::new(
        RiskPersistenceSnapshot::try_new(
            vec![original.risk(created.record.id()).unwrap().clone()],
            vec![],
            vec![],
        )
        .unwrap(),
        vec![RiskH2aTerminalReplay::new(
            RiskH2aTerminalOperation::Close,
            created.record.id().clone(),
            prepared,
            approval.clone(),
            context("risk-h2a-close-execute", "risk-h2a-close-correlation"),
            original_error.clone(),
            original.audit_events().last().unwrap().clone(),
        )],
    )
    .decode()
    .unwrap();

    policy.set(true);
    let mut reopened = InMemoryRiskService::rehydrate_with_h2a(
        FixedClock,
        TestIds(ids),
        AllowApproval,
        MutablePolicy(policy),
        AllowRiskEvidence,
        RecordedRiskClassification,
        decoded,
    )
    .unwrap();
    let audits_before_replay = reopened.audit_events().len();
    assert_eq!(
        reopened
            .approve_and_execute_close_risk(ApproveAndExecuteCloseRisk {
                approval,
                context: context("risk-h2a-close-execute", "risk-h2a-close-replay"),
            })
            .unwrap_err(),
        original_error
    );
    assert_eq!(reopened.audit_events().len(), audits_before_replay);
}
