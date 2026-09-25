use std::cell::Cell;
use std::rc::Rc;

use pmc_domain::actions::{
    ActionEvidenceAuthorityError, ActionEvidenceAuthorityPort, ActionExecutionPolicy,
    ActionExecutionPolicyPort, ActionOperationContext, ActionPersistenceCommand,
    ActionPersistenceDecodeInput, ActionPersistenceResult, ActionPersistenceSnapshot,
    ActionPersistenceTerminalCause, ActionRehydrationError, ActionReplayCapsule,
    ActionRequestRecord, ActionServiceIdSource, ApproveAndExecuteAcceptActionRequest,
    ApproveAndExecuteCancelAction, ApproveAndExecuteCompleteAction, ApproveAndExecuteReopenAction,
    CreateActionRequestDraft, DeclineActionRequest, DenyActionEvidenceAuthority,
    InMemoryActionService, LinkActionCompletionEvidence, PrepareAcceptActionRequest,
    PrepareCancelAction, PrepareCompleteAction, PrepareReopenAction, PreparedDisposition,
    StartAction, SubmitActionRequest,
};
use pmc_domain::audit::AuditActor;
use pmc_domain::classification::DataClassification;
use pmc_domain::identity::{
    ActionId, ActionRequestId, AggregateVersion, ApprovalReceiptId, AuditEventId, CorrelationId,
    EvidenceReferenceId, IdempotencyId, PreparedIntentId, StakeholderId,
};
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::work_management::{
    ActionReopenMode, ActionRequestState, ApprovalAuthorizationPort, ApprovalConfirmation,
    EvidenceReferenceMetadata, EvidenceRole, EvidenceVerification, HumanJudgment,
    HumanJudgmentDisposition, IntegrityDigest, WorkManagementApproval, WorkManagementOperation,
};

fn validated(snapshot: ActionPersistenceSnapshot) -> ActionPersistenceSnapshot {
    ActionPersistenceSnapshot::try_new(
        snapshot.requests().to_vec(),
        snapshot.actions().to_vec(),
        snapshot.prepared().to_vec(),
        snapshot.discarded_prepared().to_vec(),
        snapshot.replay().to_vec(),
        snapshot.audits().to_vec(),
    )
    .unwrap()
}

#[test]
fn terminal_h2_digest_mismatch_rehydrates_discarded_intent_and_replays_without_execution() {
    let mut service = Service::new(FixedClock, TestIds(Cell::new(0)), Allow, Allow, Allow);
    let created = service
        .create_action_request_draft(CreateActionRequestDraft {
            id: ActionRequestId::parse("request-red-digest").unwrap(),
            title: pmc_domain::BoundedText::parse("Synthetic digest request".to_owned()).unwrap(),
            details: pmc_domain::BoundedText::parse("Synthetic digest details".to_owned()).unwrap(),
            intended_owner: Some(StakeholderId::parse("owner-red-digest").unwrap()),
            response_due_at: None,
            intended_action_due_at: Some(UtcTimestamp::from_unix_millis(900)),
            classification: DataClassification::Internal,
            context: context("red-digest-create", "red-digest-create-correlation"),
        })
        .unwrap();
    let opened = service
        .submit_action_request(SubmitActionRequest {
            request_id: created.record.id().clone(),
            expected_version: created.record.version(),
            context: context("red-digest-submit", "red-digest-submit-correlation"),
        })
        .unwrap();
    let prepared = service
        .prepare_accept_action_request(PrepareAcceptActionRequest {
            request_id: opened.record.id().clone(),
            expected_version: opened.record.version(),
            context: context("red-digest-prepare", "red-digest-prepare-correlation"),
        })
        .unwrap();
    assert!(service.discard_prepared_intent(prepared.id()));
    let second = service
        .create_action_request_draft(CreateActionRequestDraft {
            id: ActionRequestId::parse("request-red-digest-2").unwrap(),
            title: pmc_domain::BoundedText::parse("Synthetic second request".to_owned()).unwrap(),
            details: pmc_domain::BoundedText::parse("Synthetic second details".to_owned()).unwrap(),
            intended_owner: Some(StakeholderId::parse("owner-red-digest-2").unwrap()),
            response_due_at: None,
            intended_action_due_at: Some(UtcTimestamp::from_unix_millis(900)),
            classification: DataClassification::Internal,
            context: context("red-digest-create-2", "red-digest-create-2-correlation"),
        })
        .unwrap();
    let second_opened = service
        .submit_action_request(SubmitActionRequest {
            request_id: second.record.id().clone(),
            expected_version: second.record.version(),
            context: context("red-digest-submit-2", "red-digest-submit-2-correlation"),
        })
        .unwrap();
    let second_prepared = service
        .prepare_accept_action_request(PrepareAcceptActionRequest {
            request_id: second_opened.record.id().clone(),
            expected_version: second_opened.record.version(),
            context: context("red-digest-prepare-2", "red-digest-prepare-2-correlation"),
        })
        .unwrap();
    let approval = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        second_prepared.payload_digest().clone(),
        IdempotencyId::parse("red-digest-execute").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    let original_error = service
        .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
            approval: approval.clone(),
            context: context("red-digest-execute", "red-digest-execute-correlation"),
        })
        .unwrap_err();
    assert_eq!(
        original_error.code(),
        pmc_domain::error::ErrorCode::SecurityPreviewExpiredOrChanged
    );
    let snapshot = validated(service.persistence_snapshot().unwrap());
    let mut reopened = Service::rehydrate(
        FixedClock,
        TestIds(Cell::new(100)),
        Allow,
        Allow,
        Allow,
        snapshot,
    );
    let audit_count = reopened.audit_events().len();
    let replayed_error = reopened
        .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
            approval,
            context: context("red-digest-execute", "red-digest-replay-correlation"),
        })
        .unwrap_err();
    assert_eq!(replayed_error, original_error);
    assert_eq!(reopened.audit_events().len(), audit_count);
    assert!(reopened
        .action(&ActionId::parse("action-red-digest").unwrap())
        .is_none());
}

#[test]
fn retryable_preview_failure_retains_intent_and_replays_exact_error_without_new_audit() {
    let mut service = Service::new(FixedClock, TestIds(Cell::new(0)), Allow, Allow, Allow);
    let created = service
        .create_action_request_draft(CreateActionRequestDraft {
            id: ActionRequestId::parse("request-red-retry").unwrap(),
            title: pmc_domain::BoundedText::parse("Synthetic retry request".to_owned()).unwrap(),
            details: pmc_domain::BoundedText::parse("Synthetic retry details".to_owned()).unwrap(),
            intended_owner: Some(StakeholderId::parse("owner-red-retry").unwrap()),
            response_due_at: None,
            intended_action_due_at: Some(UtcTimestamp::from_unix_millis(900)),
            classification: DataClassification::Internal,
            context: context("red-retry-create", "red-retry-create-correlation"),
        })
        .unwrap();
    let opened = service
        .submit_action_request(SubmitActionRequest {
            request_id: created.record.id().clone(),
            expected_version: created.record.version(),
            context: context("red-retry-submit", "red-retry-submit-correlation"),
        })
        .unwrap();
    let prepared = service
        .prepare_accept_action_request(PrepareAcceptActionRequest {
            request_id: opened.record.id().clone(),
            expected_version: opened.record.version(),
            context: context("red-retry-prepare", "red-retry-prepare-correlation"),
        })
        .unwrap();
    let approval = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("red-retry-execute").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    service
        .decline_action_request(DeclineActionRequest {
            request_id: opened.record.id().clone(),
            expected_version: opened.record.version(),
            rationale: pmc_domain::BoundedText::parse("Synthetic changed state".to_owned())
                .unwrap(),
            context: context("red-retry-change", "red-retry-change-correlation"),
        })
        .unwrap();
    let error = service
        .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
            approval: approval.clone(),
            context: context("red-retry-execute", "red-retry-fail-correlation"),
        })
        .unwrap_err();
    assert_eq!(
        error.code(),
        pmc_domain::error::ErrorCode::SecurityPreviewExpiredOrChanged
    );
    assert_eq!(
        error.correlation_id().as_str(),
        "red-retry-fail-correlation"
    );
    let audit_count = service.audit_events().len();
    let immediate = service
        .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
            approval: approval.clone(),
            context: context("red-retry-execute", "red-retry-immediate-correlation"),
        })
        .unwrap_err();
    assert_eq!(immediate, error);
    assert_eq!(service.audit_events().len(), audit_count);
    let snapshot = validated(service.persistence_snapshot().unwrap());
    assert_eq!(snapshot.prepared(), [prepared]);
    let terminal = snapshot
        .replay()
        .iter()
        .find_map(|capsule| match capsule.result() {
            ActionPersistenceResult::Terminal {
                cause,
                error,
                prepared_disposition,
                ..
            } => Some((cause, error, prepared_disposition)),
            _ => None,
        })
        .expect("preview expiry must persist one typed terminal outcome");
    assert_eq!(
        terminal.0,
        &ActionPersistenceTerminalCause::PreparedIntentChanged
    );
    assert_eq!(terminal.1, &error);
    assert_eq!(terminal.2, &PreparedDisposition::Retained);
    let mut reopened = Service::rehydrate(
        FixedClock,
        TestIds(Cell::new(100)),
        Allow,
        Allow,
        Allow,
        snapshot,
    );
    let replayed =
        reopened.approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
            approval,
            context: context("red-retry-execute", "red-retry-replay-correlation"),
        });
    assert_eq!(replayed.unwrap_err(), error);
    assert_eq!(reopened.audit_events().len(), audit_count);
}

#[test]
fn expired_terminal_rehydrates_exact_error_and_replays_without_new_effect() {
    let now = Rc::new(Cell::new(100_i64));
    let clock = MutableClock(now.clone());
    let mut service =
        ExpiringService::new(clock.clone(), TestIds(Cell::new(0)), Allow, Allow, Allow);
    let created = service
        .create_action_request_draft(CreateActionRequestDraft {
            id: ActionRequestId::parse("request-expired-terminal").unwrap(),
            title: pmc_domain::BoundedText::parse("Synthetic expired request".to_owned()).unwrap(),
            details: pmc_domain::BoundedText::parse("Synthetic expired details".to_owned())
                .unwrap(),
            intended_owner: Some(StakeholderId::parse("owner-expired-terminal").unwrap()),
            response_due_at: None,
            intended_action_due_at: Some(UtcTimestamp::from_unix_millis(900)),
            classification: DataClassification::Internal,
            context: context(
                "expired-terminal-create",
                "expired-terminal-create-correlation",
            ),
        })
        .unwrap();
    let opened = service
        .submit_action_request(SubmitActionRequest {
            request_id: created.record.id().clone(),
            expected_version: created.record.version(),
            context: context(
                "expired-terminal-submit",
                "expired-terminal-submit-correlation",
            ),
        })
        .unwrap();
    let prepared = service
        .prepare_accept_action_request(PrepareAcceptActionRequest {
            request_id: opened.record.id().clone(),
            expected_version: opened.record.version(),
            context: context(
                "expired-terminal-prepare",
                "expired-terminal-prepare-correlation",
            ),
        })
        .unwrap();
    let approval = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("expired-terminal-execute").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    now.set(300_100);
    let error = service
        .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
            approval: approval.clone(),
            context: context(
                "expired-terminal-execute",
                "expired-terminal-original-correlation",
            ),
        })
        .unwrap_err();
    assert_eq!(
        error.code(),
        pmc_domain::error::ErrorCode::SecurityPreviewExpiredOrChanged
    );
    let audit_count = service.audit_events().len();
    let snapshot = validated(service.persistence_snapshot().unwrap());
    assert!(snapshot.replay().iter().any(|capsule| matches!(
        capsule.result(),
        ActionPersistenceResult::Terminal {
            cause: ActionPersistenceTerminalCause::Expired,
            error: persisted,
            prepared_disposition: PreparedDisposition::ConsumedAndDiscarded,
            ..
        } if persisted == &error
    )));
    let mut reopened = ExpiringService::rehydrate(
        MutableClock(Rc::new(Cell::new(999_999))),
        TestIds(Cell::new(100)),
        Allow,
        Allow,
        Allow,
        snapshot,
    );
    let replayed = reopened
        .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
            approval,
            context: context(
                "expired-terminal-execute",
                "expired-terminal-replay-correlation",
            ),
        })
        .unwrap_err();
    assert_eq!(replayed, error);
    assert_eq!(reopened.audit_events().len(), audit_count);
}

#[derive(Clone)]
struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(100)
    }
}

#[derive(Clone)]
struct MutableClock(Rc<Cell<i64>>);
impl Clock for MutableClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(self.0.get())
    }
}

#[derive(Clone)]
struct TestIds(Cell<u64>);
impl ActionServiceIdSource for TestIds {
    fn next_action_id(&mut self) -> Result<ActionId, pmc_domain::DomainValueError> {
        self.0.set(self.0.get() + 1);
        ActionId::parse(format!("action-rehydrate-{}", self.0.get()))
    }

    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<PreparedIntentId, pmc_domain::DomainValueError> {
        self.0.set(self.0.get() + 1);
        PreparedIntentId::parse(format!("prepared-rehydrate-{}", self.0.get()))
    }

    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<ApprovalReceiptId, pmc_domain::DomainValueError> {
        self.0.set(self.0.get() + 1);
        ApprovalReceiptId::parse(format!("receipt-rehydrate-{}", self.0.get()))
    }

    fn next_audit_event_id(&mut self) -> Result<AuditEventId, pmc_domain::DomainValueError> {
        self.0.set(self.0.get() + 1);
        AuditEventId::parse(format!("audit-rehydrate-{}", self.0.get()))
    }
}

#[derive(Clone)]
struct Allow;
impl ApprovalAuthorizationPort for Allow {
    fn authorize(&self, actor: AuditActor) -> bool {
        actor == AuditActor::HeadOfProducts
    }
}
impl ActionExecutionPolicyPort for Allow {
    fn current_policy(&self, _: &WorkManagementOperation) -> ActionExecutionPolicy {
        ActionExecutionPolicy::Allowed
    }
}
impl ActionEvidenceAuthorityPort for Allow {
    fn resolve(
        &self,
        id: &pmc_domain::identity::EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, ActionEvidenceAuthorityError> {
        if matches!(id.as_str(), "evidence-completion" | "evidence-completion-2") {
            Ok(EvidenceReferenceMetadata::new(
                id.clone(),
                AggregateVersion::initial(),
                DataClassification::Confidential,
                EvidenceRole::ActionCompletion,
                EvidenceVerification::Verified {
                    verified_at: UtcTimestamp::from_unix_millis(110),
                    integrity_digest: IntegrityDigest::parse("a".repeat(64)).unwrap(),
                },
            ))
        } else {
            DenyActionEvidenceAuthority.resolve(id)
        }
    }
}

type Service = InMemoryActionService<FixedClock, TestIds, Allow, Allow, Allow>;
type ExpiringService = InMemoryActionService<MutableClock, TestIds, Allow, Allow, Allow>;

fn context(idempotency: impl AsRef<str>, correlation: impl AsRef<str>) -> ActionOperationContext {
    ActionOperationContext {
        idempotency_id: IdempotencyId::parse(idempotency.as_ref()).unwrap(),
        correlation_id: CorrelationId::parse(correlation.as_ref()).unwrap(),
    }
}

#[test]
fn created_request_survives_validated_rehydration_and_exactly_replays() {
    let mut original = Service::new(FixedClock, TestIds(Cell::new(0)), Allow, Allow, Allow);
    let command = CreateActionRequestDraft {
        id: ActionRequestId::parse("request-rehydrate").unwrap(),
        title: pmc_domain::BoundedText::parse("Synthetic request".to_owned()).unwrap(),
        details: pmc_domain::BoundedText::parse("Synthetic request details".to_owned()).unwrap(),
        intended_owner: Some(StakeholderId::parse("owner-rehydrate").unwrap()),
        response_due_at: Some(UtcTimestamp::from_unix_millis(500)),
        intended_action_due_at: Some(UtcTimestamp::from_unix_millis(900)),
        classification: DataClassification::Internal,
        context: context("create-request", "original-correlation"),
    };
    let expected = original
        .create_action_request_draft(command.clone())
        .unwrap();

    let exported = original.persistence_snapshot().unwrap();
    let validated = ActionPersistenceSnapshot::try_new(
        exported.requests().to_vec(),
        exported.actions().to_vec(),
        exported.prepared().to_vec(),
        exported.discarded_prepared().to_vec(),
        exported.replay().to_vec(),
        exported.audits().to_vec(),
    )
    .unwrap();
    let mut restored = Service::rehydrate(
        FixedClock,
        TestIds(Cell::new(100)),
        Allow,
        Allow,
        Allow,
        validated,
    );
    let audit_count = restored.audit_events().len();

    let replayed = restored
        .create_action_request_draft(CreateActionRequestDraft {
            context: context("create-request", "new-correlation"),
            ..command
        })
        .unwrap();

    assert_eq!(replayed, expected);
    assert_eq!(
        restored.request(replayed.record.id()),
        Some(&expected.record)
    );
    assert_eq!(restored.audit_events().len(), audit_count);
}

#[test]
fn persisted_created_draft_constructor_participates_in_create_replay_validation() {
    let mut original = Service::new(FixedClock, TestIds(Cell::new(0)), Allow, Allow, Allow);
    let command = CreateActionRequestDraft {
        id: ActionRequestId::parse("request-constructor").unwrap(),
        title: pmc_domain::BoundedText::parse("Synthetic constructor request".to_owned()).unwrap(),
        details: pmc_domain::BoundedText::parse("Synthetic constructor details".to_owned())
            .unwrap(),
        intended_owner: Some(StakeholderId::parse("owner-constructor").unwrap()),
        response_due_at: Some(UtcTimestamp::from_unix_millis(500)),
        intended_action_due_at: Some(UtcTimestamp::from_unix_millis(900)),
        classification: DataClassification::Internal,
        context: context("constructor-create", "constructor-correlation"),
    };
    let created = original
        .create_action_request_draft(command.clone())
        .unwrap();
    let snapshot = validated(original.persistence_snapshot().unwrap());
    let persisted = ActionRequestRecord::from_persisted_created_draft(
        command.id,
        command.title,
        command.details,
        command.intended_owner,
        command.response_due_at,
        command.intended_action_due_at,
        command.classification,
    );
    assert_eq!(persisted, created.record);

    let decoded = ActionPersistenceDecodeInput::new(
        vec![persisted],
        snapshot.actions().to_vec(),
        snapshot.prepared().to_vec(),
        snapshot.discarded_prepared().to_vec(),
        snapshot.replay().to_vec(),
        snapshot.audits().to_vec(),
    )
    .decode()
    .unwrap();
    assert_eq!(decoded.requests(), &[created.record]);
}

#[test]
fn persisted_created_draft_decode_rejects_mismatched_non_draft_record() {
    let mut original = Service::new(FixedClock, TestIds(Cell::new(0)), Allow, Allow, Allow);
    let created = original
        .create_action_request_draft(CreateActionRequestDraft {
            id: ActionRequestId::parse("request-constructor-mismatch").unwrap(),
            title: pmc_domain::BoundedText::parse("Synthetic mismatch request".to_owned()).unwrap(),
            details: pmc_domain::BoundedText::parse("Synthetic mismatch details".to_owned())
                .unwrap(),
            intended_owner: Some(StakeholderId::parse("owner-constructor-mismatch").unwrap()),
            response_due_at: None,
            intended_action_due_at: Some(UtcTimestamp::from_unix_millis(900)),
            classification: DataClassification::Internal,
            context: context(
                "constructor-mismatch-create",
                "constructor-mismatch-correlation",
            ),
        })
        .unwrap();
    let opened = original
        .submit_action_request(SubmitActionRequest {
            request_id: created.record.id().clone(),
            expected_version: created.record.version(),
            context: context("constructor-mismatch-submit", "constructor-mismatch-submit"),
        })
        .unwrap();
    let snapshot = validated(original.persistence_snapshot().unwrap());
    let mut create_only_replay = snapshot.replay().to_vec();
    create_only_replay.truncate(1);
    let mut create_only_audits = snapshot.audits().to_vec();
    create_only_audits.truncate(1);

    let result = ActionPersistenceDecodeInput::new(
        vec![opened.record],
        snapshot.actions().to_vec(),
        snapshot.prepared().to_vec(),
        snapshot.discarded_prepared().to_vec(),
        create_only_replay,
        create_only_audits,
    )
    .decode();
    assert!(result.is_err());
}

#[test]
fn persisted_submitted_open_constructor_rehydrates_create_submit_exactly() {
    let mut original = Service::new(FixedClock, TestIds(Cell::new(0)), Allow, Allow, Allow);
    let create = CreateActionRequestDraft {
        id: ActionRequestId::parse("request-submitted-constructor").unwrap(),
        title: pmc_domain::BoundedText::parse("Synthetic submitted request".to_owned()).unwrap(),
        details: pmc_domain::BoundedText::parse("Synthetic submitted details".to_owned()).unwrap(),
        intended_owner: Some(StakeholderId::parse("owner-submitted-constructor").unwrap()),
        response_due_at: Some(UtcTimestamp::from_unix_millis(500)),
        intended_action_due_at: Some(UtcTimestamp::from_unix_millis(900)),
        classification: DataClassification::Internal,
        context: context(
            "submitted-constructor-create",
            "submitted-constructor-create-corr",
        ),
    };
    let created = original.create_action_request_draft(create).unwrap();
    let submitted = original
        .submit_action_request(SubmitActionRequest {
            request_id: created.record.id().clone(),
            expected_version: created.record.version(),
            context: context(
                "submitted-constructor-submit",
                "submitted-constructor-submit-corr",
            ),
        })
        .unwrap();

    let exported = validated(original.persistence_snapshot().unwrap());
    let persisted = ActionRequestRecord::from_persisted_submitted_open(
        created.record.id().clone(),
        created.record.title().clone(),
        created.record.details().clone(),
        created.record.intended_owner().cloned(),
        created.record.response_due_at(),
        created.record.intended_action_due_at(),
        created.record.classification(),
    );
    assert_eq!(persisted.state(), ActionRequestState::Open);
    assert_eq!(persisted.version(), AggregateVersion::new(2).unwrap());
    assert!(persisted.terminal_rationale().is_none());
    assert!(persisted.linked_action_id().is_none());
    assert!(persisted.source_decision_id().is_none());
    assert!(!persisted.has_superseded_premise());

    let decoded = ActionPersistenceDecodeInput::new(
        vec![persisted],
        exported.actions().to_vec(),
        exported.prepared().to_vec(),
        exported.discarded_prepared().to_vec(),
        exported.replay().to_vec(),
        exported.audits().to_vec(),
    )
    .decode()
    .unwrap();
    assert_eq!(decoded.requests(), &[submitted.record]);
    assert_eq!(decoded.replay(), exported.replay());
    assert_eq!(decoded.audits(), exported.audits());
}

#[test]
fn persisted_submitted_open_decode_rejects_tampered_final_state_or_version() {
    let mut original = Service::new(FixedClock, TestIds(Cell::new(0)), Allow, Allow, Allow);
    let create = CreateActionRequestDraft {
        id: ActionRequestId::parse("request-submitted-tamper").unwrap(),
        title: pmc_domain::BoundedText::parse("Synthetic tamper request".to_owned()).unwrap(),
        details: pmc_domain::BoundedText::parse("Synthetic tamper details".to_owned()).unwrap(),
        intended_owner: None,
        response_due_at: None,
        intended_action_due_at: None,
        classification: DataClassification::Public,
        context: context("submitted-tamper-create", "submitted-tamper-create-corr"),
    };
    let created = original
        .create_action_request_draft(create.clone())
        .unwrap();
    original
        .submit_action_request(SubmitActionRequest {
            request_id: created.record.id().clone(),
            expected_version: created.record.version(),
            context: context("submitted-tamper-submit", "submitted-tamper-submit-corr"),
        })
        .unwrap();
    let exported = validated(original.persistence_snapshot().unwrap());

    let draft = ActionRequestRecord::from_persisted_created_draft(
        create.id,
        create.title,
        create.details,
        create.intended_owner,
        create.response_due_at,
        create.intended_action_due_at,
        create.classification,
    );
    let state_tampered = ActionPersistenceDecodeInput::new(
        vec![draft],
        exported.actions().to_vec(),
        exported.prepared().to_vec(),
        exported.discarded_prepared().to_vec(),
        exported.replay().to_vec(),
        exported.audits().to_vec(),
    )
    .decode();
    assert!(state_tampered.is_err());

    let mut replay = exported.replay().to_vec();
    let submit_index = replay
        .iter()
        .position(|capsule| {
            matches!(
                capsule.command(),
                ActionPersistenceCommand::TransitionRequest {
                    target_state: ActionRequestState::Open,
                    ..
                }
            )
        })
        .expect("create→submit replay must be present");
    let original_submit = replay[submit_index].clone();
    let mut tampered_command = original_submit.command().clone();
    if let ActionPersistenceCommand::TransitionRequest {
        expected_version, ..
    } = &mut tampered_command
    {
        *expected_version = AggregateVersion::new(2).unwrap();
    }
    replay[submit_index] = ActionReplayCapsule::new(
        original_submit.idempotency_id().clone(),
        original_submit.original_correlation_id().clone(),
        original_submit.operation_ordinal(),
        tampered_command,
        original_submit.result().clone(),
        original_submit.audit_event_ids().to_vec(),
    );
    let persisted = ActionRequestRecord::from_persisted_submitted_open(
        created.record.id().clone(),
        created.record.title().clone(),
        created.record.details().clone(),
        created.record.intended_owner().cloned(),
        created.record.response_due_at(),
        created.record.intended_action_due_at(),
        created.record.classification(),
    );
    let version_tampered = ActionPersistenceDecodeInput::new(
        vec![persisted],
        exported.actions().to_vec(),
        exported.prepared().to_vec(),
        exported.discarded_prepared().to_vec(),
        replay,
        exported.audits().to_vec(),
    )
    .decode();
    assert!(matches!(
        version_tampered,
        Err(ActionRehydrationError::CommandResultMismatch)
    ));
}

#[test]
fn request_lifecycle_survives_rehydration_and_every_command_replays_exactly() {
    let mut original = Service::new(FixedClock, TestIds(Cell::new(0)), Allow, Allow, Allow);
    let create = CreateActionRequestDraft {
        id: ActionRequestId::parse("request-lifecycle-rehydrate").unwrap(),
        title: pmc_domain::BoundedText::parse("Synthetic lifecycle request".to_owned()).unwrap(),
        details: pmc_domain::BoundedText::parse("Synthetic lifecycle details".to_owned()).unwrap(),
        intended_owner: Some(StakeholderId::parse("owner-lifecycle-rehydrate").unwrap()),
        response_due_at: Some(UtcTimestamp::from_unix_millis(500)),
        intended_action_due_at: Some(UtcTimestamp::from_unix_millis(900)),
        classification: DataClassification::Internal,
        context: context("create-lifecycle", "create-correlation"),
    };
    let created = original
        .create_action_request_draft(create.clone())
        .unwrap();
    let submit = SubmitActionRequest {
        request_id: created.record.id().clone(),
        expected_version: created.record.version(),
        context: context("submit-lifecycle", "submit-correlation"),
    };
    let submitted = original.submit_action_request(submit.clone()).unwrap();
    let decline = DeclineActionRequest {
        request_id: submitted.record.id().clone(),
        expected_version: submitted.record.version(),
        rationale: pmc_domain::BoundedText::parse("Synthetic capacity is unavailable".to_owned())
            .unwrap(),
        context: context("decline-lifecycle", "decline-correlation"),
    };
    let declined = original.decline_action_request(decline.clone()).unwrap();
    assert_eq!(declined.record.state(), ActionRequestState::Declined);

    let exported = original.persistence_snapshot().unwrap();
    let validated = ActionPersistenceSnapshot::try_new(
        exported.requests().to_vec(),
        exported.actions().to_vec(),
        exported.prepared().to_vec(),
        exported.discarded_prepared().to_vec(),
        exported.replay().to_vec(),
        exported.audits().to_vec(),
    )
    .unwrap();
    let mut restored = Service::rehydrate(
        FixedClock,
        TestIds(Cell::new(100)),
        Allow,
        Allow,
        Allow,
        validated,
    );
    let audit_count = restored.audit_events().len();

    let replayed_create = restored
        .create_action_request_draft(CreateActionRequestDraft {
            context: context("create-lifecycle", "new-create-correlation"),
            ..create
        })
        .unwrap();
    let replayed_submit = restored
        .submit_action_request(SubmitActionRequest {
            context: context("submit-lifecycle", "new-submit-correlation"),
            ..submit
        })
        .unwrap();
    let replayed_decline = restored
        .decline_action_request(DeclineActionRequest {
            context: context("decline-lifecycle", "new-decline-correlation"),
            ..decline
        })
        .unwrap();

    assert_eq!(replayed_create, created);
    assert_eq!(replayed_submit, submitted);
    assert_eq!(replayed_decline, declined);
    assert_eq!(
        restored.request(declined.record.id()),
        Some(&declined.record)
    );
    assert_eq!(restored.audit_events().len(), audit_count);
}

#[test]
fn pending_accept_survives_reopen_without_execution_then_executes_and_replays_once() {
    let mut original = Service::new(FixedClock, TestIds(Cell::new(0)), Allow, Allow, Allow);
    let created = original
        .create_action_request_draft(CreateActionRequestDraft {
            id: ActionRequestId::parse("request-pending-accept").unwrap(),
            title: pmc_domain::BoundedText::parse("Synthetic pending request".to_owned()).unwrap(),
            details: pmc_domain::BoundedText::parse("Synthetic pending details".to_owned())
                .unwrap(),
            intended_owner: Some(StakeholderId::parse("owner-pending-accept").unwrap()),
            response_due_at: Some(UtcTimestamp::from_unix_millis(500)),
            intended_action_due_at: Some(UtcTimestamp::from_unix_millis(900)),
            classification: DataClassification::Internal,
            context: context("create-pending", "create-pending-correlation"),
        })
        .unwrap();
    let opened = original
        .submit_action_request(SubmitActionRequest {
            request_id: created.record.id().clone(),
            expected_version: created.record.version(),
            context: context("submit-pending", "submit-pending-correlation"),
        })
        .unwrap();
    let prepared = original
        .prepare_accept_action_request(PrepareAcceptActionRequest {
            request_id: opened.record.id().clone(),
            expected_version: opened.record.version(),
            context: context("prepare-pending", "prepare-pending-correlation"),
        })
        .unwrap();
    let action_id = match prepared.operation() {
        WorkManagementOperation::AcceptActionRequest { action_id, .. } => action_id.clone(),
        _ => panic!("accept preparation must retain its typed operation"),
    };

    let exported = original.persistence_snapshot().unwrap();
    let validated = ActionPersistenceSnapshot::try_new(
        exported.requests().to_vec(),
        exported.actions().to_vec(),
        exported.prepared().to_vec(),
        exported.discarded_prepared().to_vec(),
        exported.replay().to_vec(),
        exported.audits().to_vec(),
    )
    .unwrap();
    let mut restored = Service::rehydrate(
        FixedClock,
        TestIds(Cell::new(100)),
        Allow,
        Allow,
        Allow,
        validated,
    );
    assert!(restored.action(&action_id).is_none());
    let approval = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("execute-pending").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    let execute = ApproveAndExecuteAcceptActionRequest {
        approval,
        context: context("execute-pending", "execute-pending-correlation"),
    };
    let accepted = restored
        .approve_and_execute_accept_action_request(execute.clone())
        .unwrap();
    let audit_count = restored.audit_events().len();
    let replayed = restored
        .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
            context: context("execute-pending", "new-execute-correlation"),
            ..execute.clone()
        })
        .unwrap();

    assert_eq!(replayed, accepted);
    assert_eq!(restored.action(&action_id), Some(&accepted.action));
    assert_eq!(restored.audit_events().len(), audit_count);

    let terminal = restored.persistence_snapshot().unwrap();
    let terminal = ActionPersistenceSnapshot::try_new(
        terminal.requests().to_vec(),
        terminal.actions().to_vec(),
        terminal.prepared().to_vec(),
        terminal.discarded_prepared().to_vec(),
        terminal.replay().to_vec(),
        terminal.audits().to_vec(),
    )
    .unwrap();
    let mut reopened_terminal = Service::rehydrate(
        FixedClock,
        TestIds(Cell::new(200)),
        Allow,
        Allow,
        Allow,
        terminal,
    );
    let reopened_audit_count = reopened_terminal.audit_events().len();
    let reopened_replay = reopened_terminal
        .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
            context: context("execute-pending", "post-terminal-reopen-correlation"),
            ..execute
        })
        .unwrap();
    assert_eq!(reopened_replay, accepted);
    assert_eq!(reopened_terminal.action(&action_id), Some(&accepted.action));
    assert_eq!(reopened_terminal.audit_events().len(), reopened_audit_count);
}

#[test]
fn ordinary_action_persistence_rehydrates_start_and_evidence_link_exactly() {
    let mut original = Service::new(FixedClock, TestIds(Cell::new(0)), Allow, Allow, Allow);
    let created = original
        .create_action_request_draft(CreateActionRequestDraft {
            id: ActionRequestId::parse("request-ordinary-persistence").unwrap(),
            title: pmc_domain::BoundedText::parse("Synthetic ordinary action".to_owned()).unwrap(),
            details: pmc_domain::BoundedText::parse("Synthetic ordinary details".to_owned())
                .unwrap(),
            intended_owner: Some(StakeholderId::parse("owner-ordinary-persistence").unwrap()),
            response_due_at: None,
            intended_action_due_at: Some(UtcTimestamp::from_unix_millis(900)),
            classification: DataClassification::Internal,
            context: context("ordinary-create", "ordinary-create-correlation"),
        })
        .unwrap();
    let opened = original
        .submit_action_request(SubmitActionRequest {
            request_id: created.record.id().clone(),
            expected_version: created.record.version(),
            context: context("ordinary-submit", "ordinary-submit-correlation"),
        })
        .unwrap();
    let prepared = original
        .prepare_accept_action_request(PrepareAcceptActionRequest {
            request_id: opened.record.id().clone(),
            expected_version: opened.record.version(),
            context: context("ordinary-prepare", "ordinary-prepare-correlation"),
        })
        .unwrap();
    let accepted = original
        .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
            approval: WorkManagementApproval::new(
                prepared.id().clone(),
                AuditActor::HeadOfProducts,
                prepared.payload_digest().clone(),
                IdempotencyId::parse("ordinary-accept").unwrap(),
                Some(ApprovalConfirmation::Confirmed),
            )
            .unwrap(),
            context: context("ordinary-accept", "ordinary-accept-correlation"),
        })
        .unwrap();
    let started = original
        .start_action(StartAction {
            action_id: accepted.action.id().clone(),
            expected_version: accepted.action.version(),
            context: context("ordinary-start", "ordinary-start-correlation"),
        })
        .unwrap();
    let linked = original
        .link_action_completion_evidence(LinkActionCompletionEvidence {
            action_id: started.record.id().clone(),
            expected_version: started.record.version(),
            evidence_id: pmc_domain::identity::EvidenceReferenceId::parse("evidence-completion")
                .unwrap(),
            context: context("ordinary-link", "ordinary-link-correlation"),
        })
        .unwrap();

    let exported = original.persistence_snapshot().unwrap();
    assert_eq!(exported.replay().len(), 6);
    let validated = ActionPersistenceSnapshot::try_new(
        exported.requests().to_vec(),
        exported.actions().to_vec(),
        exported.prepared().to_vec(),
        exported.discarded_prepared().to_vec(),
        exported.replay().to_vec(),
        exported.audits().to_vec(),
    )
    .unwrap();
    let mut restored = Service::rehydrate(
        FixedClock,
        TestIds(Cell::new(100)),
        Allow,
        Allow,
        Allow,
        validated,
    );
    let audit_count = restored.audit_events().len();
    let replayed_start = restored
        .start_action(StartAction {
            action_id: accepted.action.id().clone(),
            expected_version: accepted.action.version(),
            context: context("ordinary-start", "new-start-correlation"),
        })
        .unwrap();
    let replayed_link = restored
        .link_action_completion_evidence(LinkActionCompletionEvidence {
            action_id: started.record.id().clone(),
            expected_version: started.record.version(),
            evidence_id: pmc_domain::identity::EvidenceReferenceId::parse("evidence-completion")
                .unwrap(),
            context: context("ordinary-link", "new-link-correlation"),
        })
        .unwrap();
    assert_eq!(replayed_start, started);
    assert_eq!(replayed_link, linked);
    assert_eq!(restored.action(linked.record.id()), Some(&linked.record));
    assert_eq!(restored.audit_events().len(), audit_count);

    let reexported = restored.persistence_snapshot().unwrap();
    assert_eq!(
        reexported
            .replay()
            .iter()
            .map(|capsule| capsule.operation_ordinal())
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4, 5]
    );
    assert_eq!(
        reexported
            .replay()
            .iter()
            .map(|capsule| capsule.original_correlation_id().as_str())
            .collect::<Vec<_>>(),
        vec![
            "ordinary-create-correlation",
            "ordinary-submit-correlation",
            "ordinary-prepare-correlation",
            "ordinary-accept-correlation",
            "ordinary-start-correlation",
            "ordinary-link-correlation",
        ]
    );
}

#[test]
fn discarded_prepare_compacts_history_and_later_write_reopens_without_execution() {
    let mut original = Service::new(FixedClock, TestIds(Cell::new(0)), Allow, Allow, Allow);
    let created = original
        .create_action_request_draft(CreateActionRequestDraft {
            id: ActionRequestId::parse("request-discarded-prepare").unwrap(),
            title: pmc_domain::BoundedText::parse("Synthetic discard request".to_owned()).unwrap(),
            details: pmc_domain::BoundedText::parse("Synthetic discard details".to_owned())
                .unwrap(),
            intended_owner: Some(StakeholderId::parse("owner-discarded-prepare").unwrap()),
            response_due_at: None,
            intended_action_due_at: Some(UtcTimestamp::from_unix_millis(900)),
            classification: DataClassification::Internal,
            context: context("create-discard", "create-discard-correlation"),
        })
        .unwrap();
    let opened = original
        .submit_action_request(SubmitActionRequest {
            request_id: created.record.id().clone(),
            expected_version: created.record.version(),
            context: context("submit-discard", "submit-discard-correlation"),
        })
        .unwrap();
    let prepared = original
        .prepare_accept_action_request(PrepareAcceptActionRequest {
            request_id: opened.record.id().clone(),
            expected_version: opened.record.version(),
            context: context("prepare-discard", "prepare-discard-correlation"),
        })
        .unwrap();
    let discarded_action_id = match prepared.operation() {
        WorkManagementOperation::AcceptActionRequest { action_id, .. } => action_id.clone(),
        _ => panic!("accept preparation must retain its typed operation"),
    };
    assert!(original.discard_prepared_intent(prepared.id()));

    let later_command = CreateActionRequestDraft {
        id: ActionRequestId::parse("request-after-discard").unwrap(),
        title: pmc_domain::BoundedText::parse("Synthetic later request".to_owned()).unwrap(),
        details: pmc_domain::BoundedText::parse("Synthetic later details".to_owned()).unwrap(),
        intended_owner: None,
        response_due_at: None,
        intended_action_due_at: None,
        classification: DataClassification::Internal,
        context: context("create-after-discard", "create-after-discard-correlation"),
    };
    let later = original
        .create_action_request_draft(later_command.clone())
        .unwrap();

    let snapshot = validated(original.persistence_snapshot().unwrap());
    assert!(snapshot.prepared().is_empty());
    assert_eq!(snapshot.discarded_prepared(), [prepared]);
    assert_eq!(
        snapshot
            .replay()
            .iter()
            .map(|capsule| capsule.operation_ordinal())
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    let mut reopened = Service::rehydrate(
        FixedClock,
        TestIds(Cell::new(100)),
        Allow,
        Allow,
        Allow,
        snapshot,
    );
    let audit_count = reopened.audit_events().len();
    let replayed = reopened
        .create_action_request_draft(CreateActionRequestDraft {
            context: context("create-after-discard", "later-replay-correlation"),
            ..later_command
        })
        .unwrap();

    assert_eq!(replayed, later);
    assert_eq!(reopened.audit_events().len(), audit_count);
    assert!(reopened.action(&discarded_action_id).is_none());
}

#[test]
fn pending_complete_rehydrates_without_execution_then_completes_and_reopens_exactly() {
    let mut original = Service::new(FixedClock, TestIds(Cell::new(0)), Allow, Allow, Allow);
    let created = original
        .create_action_request_draft(CreateActionRequestDraft {
            id: ActionRequestId::parse("request-h2a-terminal-history").unwrap(),
            title: pmc_domain::BoundedText::parse("Synthetic terminal history".to_owned()).unwrap(),
            details: pmc_domain::BoundedText::parse("Synthetic terminal details".to_owned())
                .unwrap(),
            intended_owner: Some(StakeholderId::parse("owner-h2a-terminal").unwrap()),
            response_due_at: None,
            intended_action_due_at: Some(UtcTimestamp::from_unix_millis(900)),
            classification: DataClassification::Internal,
            context: context("h2a-create", "h2a-create-correlation"),
        })
        .unwrap();
    let opened = original
        .submit_action_request(SubmitActionRequest {
            request_id: created.record.id().clone(),
            expected_version: created.record.version(),
            context: context("h2a-submit", "h2a-submit-correlation"),
        })
        .unwrap();
    let prepared_accept = original
        .prepare_accept_action_request(PrepareAcceptActionRequest {
            request_id: opened.record.id().clone(),
            expected_version: opened.record.version(),
            context: context("h2a-prepare-accept", "h2a-prepare-accept-correlation"),
        })
        .unwrap();
    let accepted = original
        .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
            approval: WorkManagementApproval::new(
                prepared_accept.id().clone(),
                AuditActor::HeadOfProducts,
                prepared_accept.payload_digest().clone(),
                IdempotencyId::parse("h2a-accept").unwrap(),
                Some(ApprovalConfirmation::Confirmed),
            )
            .unwrap(),
            context: context("h2a-accept", "h2a-accept-correlation"),
        })
        .unwrap();
    let started = original
        .start_action(StartAction {
            action_id: accepted.action.id().clone(),
            expected_version: accepted.action.version(),
            context: context("h2a-start", "h2a-start-correlation"),
        })
        .unwrap();
    let linked = original
        .link_action_completion_evidence(LinkActionCompletionEvidence {
            action_id: started.record.id().clone(),
            expected_version: started.record.version(),
            evidence_id: pmc_domain::identity::EvidenceReferenceId::parse("evidence-completion")
                .unwrap(),
            context: context("h2a-link", "h2a-link-correlation"),
        })
        .unwrap();
    let prepared_complete = original
        .prepare_complete_action(PrepareCompleteAction {
            action_id: linked.record.id().clone(),
            expected_version: linked.record.version(),
            judgment: None,
            context: context("h2a-prepare-complete", "h2a-prepare-complete-correlation"),
        })
        .unwrap();
    let complete_approval = WorkManagementApproval::new(
        prepared_complete.id().clone(),
        AuditActor::HeadOfProducts,
        prepared_complete.payload_digest().clone(),
        IdempotencyId::parse("h2a-complete").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();

    let snapshot = original.persistence_snapshot().unwrap();
    let snapshot = ActionPersistenceSnapshot::try_new(
        snapshot.requests().to_vec(),
        snapshot.actions().to_vec(),
        snapshot.prepared().to_vec(),
        snapshot.discarded_prepared().to_vec(),
        snapshot.replay().to_vec(),
        snapshot.audits().to_vec(),
    )
    .unwrap();
    let mut reopened = Service::rehydrate(
        FixedClock,
        TestIds(Cell::new(100)),
        Allow,
        Allow,
        Allow,
        snapshot,
    );
    assert_eq!(reopened.action(linked.record.id()), Some(&linked.record));
    let audit_count = reopened.audit_events().len();
    let completed = reopened
        .approve_and_execute_complete_action(ApproveAndExecuteCompleteAction {
            approval: complete_approval.clone(),
            context: context("h2a-complete", "h2a-complete-correlation"),
        })
        .unwrap();
    assert_eq!(
        completed.record.state(),
        pmc_domain::work_management::ActionState::Completed
    );
    assert_eq!(reopened.audit_events().len(), audit_count + 1);
    let replayed_complete = reopened
        .approve_and_execute_complete_action(ApproveAndExecuteCompleteAction {
            approval: complete_approval,
            context: context("h2a-complete", "h2a-complete-replay-correlation"),
        })
        .unwrap();
    assert_eq!(replayed_complete, completed);
    assert_eq!(reopened.audit_events().len(), audit_count + 1);

    let prepared_reopen = reopened
        .prepare_reopen_action(PrepareReopenAction {
            action_id: completed.record.id().clone(),
            expected_version: completed.record.version(),
            mode: ActionReopenMode::ReopenCompleted,
            reason: pmc_domain::BoundedText::parse("Synthetic follow-up required".to_owned())
                .unwrap(),
            context: context("h2a-prepare-reopen", "h2a-prepare-reopen-correlation"),
        })
        .unwrap();
    let reopen_approval = WorkManagementApproval::new(
        prepared_reopen.id().clone(),
        AuditActor::HeadOfProducts,
        prepared_reopen.payload_digest().clone(),
        IdempotencyId::parse("h2a-reopen").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    let reopened_action = reopened
        .approve_and_execute_reopen_action(ApproveAndExecuteReopenAction {
            approval: reopen_approval.clone(),
            context: context("h2a-reopen", "h2a-reopen-correlation"),
        })
        .unwrap();
    assert_eq!(
        reopened_action.record.state(),
        pmc_domain::work_management::ActionState::InProgress
    );
    let replayed_reopen = reopened
        .approve_and_execute_reopen_action(ApproveAndExecuteReopenAction {
            approval: reopen_approval,
            context: context("h2a-reopen", "h2a-reopen-replay-correlation"),
        })
        .unwrap();
    assert_eq!(replayed_reopen, reopened_action);
    assert_eq!(reopened.audit_events().len(), audit_count + 2);
}

#[test]
fn cancelled_action_history_rehydrates_and_replays_without_new_authority() {
    let mut original = Service::new(FixedClock, TestIds(Cell::new(0)), Allow, Allow, Allow);
    let created = original
        .create_action_request_draft(CreateActionRequestDraft {
            id: ActionRequestId::parse("request-h2a-cancel-history").unwrap(),
            title: pmc_domain::BoundedText::parse("Synthetic cancellation".to_owned()).unwrap(),
            details: pmc_domain::BoundedText::parse("Synthetic cancellation details".to_owned())
                .unwrap(),
            intended_owner: Some(StakeholderId::parse("owner-h2a-cancel").unwrap()),
            response_due_at: None,
            intended_action_due_at: Some(UtcTimestamp::from_unix_millis(900)),
            classification: DataClassification::Internal,
            context: context("cancel-create", "cancel-create-correlation"),
        })
        .unwrap();
    let opened = original
        .submit_action_request(SubmitActionRequest {
            request_id: created.record.id().clone(),
            expected_version: created.record.version(),
            context: context("cancel-submit", "cancel-submit-correlation"),
        })
        .unwrap();
    let prepared_accept = original
        .prepare_accept_action_request(PrepareAcceptActionRequest {
            request_id: opened.record.id().clone(),
            expected_version: opened.record.version(),
            context: context("cancel-prepare-accept", "cancel-prepare-accept-correlation"),
        })
        .unwrap();
    let accepted = original
        .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
            approval: WorkManagementApproval::new(
                prepared_accept.id().clone(),
                AuditActor::HeadOfProducts,
                prepared_accept.payload_digest().clone(),
                IdempotencyId::parse("cancel-accept").unwrap(),
                Some(ApprovalConfirmation::Confirmed),
            )
            .unwrap(),
            context: context("cancel-accept", "cancel-accept-correlation"),
        })
        .unwrap();
    let prepared_cancel = original
        .prepare_cancel_action(PrepareCancelAction {
            action_id: accepted.action.id().clone(),
            expected_version: accepted.action.version(),
            reason: pmc_domain::BoundedText::parse("Synthetic cancellation reason".to_owned())
                .unwrap(),
            context: context("cancel-prepare", "cancel-prepare-correlation"),
        })
        .unwrap();
    let approval = WorkManagementApproval::new(
        prepared_cancel.id().clone(),
        AuditActor::HeadOfProducts,
        prepared_cancel.payload_digest().clone(),
        IdempotencyId::parse("cancel-execute").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    let cancelled = original
        .approve_and_execute_cancel_action(ApproveAndExecuteCancelAction {
            approval: approval.clone(),
            context: context("cancel-execute", "cancel-execute-correlation"),
        })
        .unwrap();
    let snapshot = original.persistence_snapshot().unwrap();
    let snapshot = ActionPersistenceSnapshot::try_new(
        snapshot.requests().to_vec(),
        snapshot.actions().to_vec(),
        snapshot.prepared().to_vec(),
        snapshot.discarded_prepared().to_vec(),
        snapshot.replay().to_vec(),
        snapshot.audits().to_vec(),
    )
    .unwrap();
    let mut restored = Service::rehydrate(
        FixedClock,
        TestIds(Cell::new(100)),
        Allow,
        Allow,
        Allow,
        snapshot,
    );
    let audit_count = restored.audit_events().len();
    let replayed = restored
        .approve_and_execute_cancel_action(ApproveAndExecuteCancelAction {
            approval,
            context: context("cancel-execute", "cancel-replay-correlation"),
        })
        .unwrap();
    assert_eq!(replayed, cancelled);
    assert_eq!(
        restored.action(cancelled.record.id()),
        Some(&cancelled.record)
    );
    assert_eq!(restored.audit_events().len(), audit_count);
}

#[test]
fn reverse_inserted_completion_evidence_rehydrates_cancel_and_reopen_histories() {
    let mut original = Service::new(FixedClock, TestIds(Cell::new(0)), Allow, Allow, Allow);
    let created = original
        .create_action_request_draft(CreateActionRequestDraft {
            id: ActionRequestId::parse("request-reverse-evidence").unwrap(),
            title: pmc_domain::BoundedText::parse("Synthetic reverse evidence".to_owned()).unwrap(),
            details: pmc_domain::BoundedText::parse("Synthetic reverse details".to_owned())
                .unwrap(),
            intended_owner: Some(StakeholderId::parse("owner-reverse-evidence").unwrap()),
            response_due_at: None,
            intended_action_due_at: Some(UtcTimestamp::from_unix_millis(900)),
            classification: DataClassification::Internal,
            context: context("reverse-create", "reverse-create-correlation"),
        })
        .unwrap();
    let opened = original
        .submit_action_request(SubmitActionRequest {
            request_id: created.record.id().clone(),
            expected_version: created.record.version(),
            context: context("reverse-submit", "reverse-submit-correlation"),
        })
        .unwrap();
    let prepared_accept = original
        .prepare_accept_action_request(PrepareAcceptActionRequest {
            request_id: opened.record.id().clone(),
            expected_version: opened.record.version(),
            context: context(
                "reverse-prepare-accept",
                "reverse-prepare-accept-correlation",
            ),
        })
        .unwrap();
    let accepted = original
        .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
            approval: WorkManagementApproval::new(
                prepared_accept.id().clone(),
                AuditActor::HeadOfProducts,
                prepared_accept.payload_digest().clone(),
                IdempotencyId::parse("reverse-accept").unwrap(),
                Some(ApprovalConfirmation::Confirmed),
            )
            .unwrap(),
            context: context("reverse-accept", "reverse-accept-correlation"),
        })
        .unwrap();
    let started = original
        .start_action(StartAction {
            action_id: accepted.action.id().clone(),
            expected_version: accepted.action.version(),
            context: context("reverse-start", "reverse-start-correlation"),
        })
        .unwrap();
    let first = original
        .link_action_completion_evidence(LinkActionCompletionEvidence {
            action_id: started.record.id().clone(),
            expected_version: started.record.version(),
            evidence_id: pmc_domain::identity::EvidenceReferenceId::parse("evidence-completion-2")
                .unwrap(),
            context: context("reverse-link-2", "reverse-link-2-correlation"),
        })
        .unwrap();
    let linked = original
        .link_action_completion_evidence(LinkActionCompletionEvidence {
            action_id: first.record.id().clone(),
            expected_version: first.record.version(),
            evidence_id: pmc_domain::identity::EvidenceReferenceId::parse("evidence-completion")
                .unwrap(),
            context: context("reverse-link-1", "reverse-link-1-correlation"),
        })
        .unwrap();
    let prepared_cancel = original
        .prepare_cancel_action(PrepareCancelAction {
            action_id: linked.record.id().clone(),
            expected_version: linked.record.version(),
            reason: pmc_domain::BoundedText::parse("Synthetic reverse cancellation".to_owned())
                .unwrap(),
            context: context(
                "reverse-prepare-cancel",
                "reverse-prepare-cancel-correlation",
            ),
        })
        .unwrap();
    let cancel_approval = WorkManagementApproval::new(
        prepared_cancel.id().clone(),
        AuditActor::HeadOfProducts,
        prepared_cancel.payload_digest().clone(),
        IdempotencyId::parse("reverse-cancel").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    let snapshot = original.persistence_snapshot().unwrap();
    let snapshot = ActionPersistenceSnapshot::try_new(
        snapshot.requests().to_vec(),
        snapshot.actions().to_vec(),
        snapshot.prepared().to_vec(),
        snapshot.discarded_prepared().to_vec(),
        snapshot.replay().to_vec(),
        snapshot.audits().to_vec(),
    )
    .unwrap();
    let mut restored = Service::rehydrate(
        FixedClock,
        TestIds(Cell::new(100)),
        Allow,
        Allow,
        Allow,
        snapshot,
    );
    let cancelled = restored
        .approve_and_execute_cancel_action(ApproveAndExecuteCancelAction {
            approval: cancel_approval,
            context: context("reverse-cancel", "reverse-cancel-correlation"),
        })
        .unwrap();
    let prepared_reopen = restored
        .prepare_reopen_action(PrepareReopenAction {
            action_id: cancelled.record.id().clone(),
            expected_version: cancelled.record.version(),
            mode: ActionReopenMode::RestartCancelled,
            reason: pmc_domain::BoundedText::parse("Synthetic reverse restart".to_owned()).unwrap(),
            context: context(
                "reverse-prepare-reopen",
                "reverse-prepare-reopen-correlation",
            ),
        })
        .unwrap();
    let reopen_approval = WorkManagementApproval::new(
        prepared_reopen.id().clone(),
        AuditActor::HeadOfProducts,
        prepared_reopen.payload_digest().clone(),
        IdempotencyId::parse("reverse-reopen").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    let reopened = restored
        .approve_and_execute_reopen_action(ApproveAndExecuteReopenAction {
            approval: reopen_approval.clone(),
            context: context("reverse-reopen", "reverse-reopen-correlation"),
        })
        .unwrap();
    let terminal = restored.persistence_snapshot().unwrap();
    let terminal = ActionPersistenceSnapshot::try_new(
        terminal.requests().to_vec(),
        terminal.actions().to_vec(),
        terminal.prepared().to_vec(),
        terminal.discarded_prepared().to_vec(),
        terminal.replay().to_vec(),
        terminal.audits().to_vec(),
    )
    .unwrap();
    let mut terminal_reopened = Service::rehydrate(
        FixedClock,
        TestIds(Cell::new(200)),
        Allow,
        Allow,
        Allow,
        terminal,
    );
    let replayed = terminal_reopened
        .approve_and_execute_reopen_action(ApproveAndExecuteReopenAction {
            approval: reopen_approval,
            context: context("reverse-reopen", "reverse-reopen-replay-correlation"),
        })
        .unwrap();
    assert_eq!(replayed, reopened);
    assert_eq!(
        terminal_reopened.action(reopened.record.id()),
        Some(&reopened.record)
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum H3Mode {
    Missing,
    EvidenceNotFound,
    Verified,
    Unverified,
    Unclassified,
    Unavailable,
}

#[derive(Clone)]
struct H3Authority(Rc<Cell<H3Mode>>);

impl ActionEvidenceAuthorityPort for H3Authority {
    fn resolve(
        &self,
        id: &EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, ActionEvidenceAuthorityError> {
        if id.as_str() != "evidence-h3-red" {
            return Err(ActionEvidenceAuthorityError::NotFound);
        }
        match self.0.get() {
            H3Mode::Missing | H3Mode::EvidenceNotFound => {
                Err(ActionEvidenceAuthorityError::NotFound)
            }
            H3Mode::Unavailable => Err(ActionEvidenceAuthorityError::Unavailable),
            H3Mode::Verified => Ok(EvidenceReferenceMetadata::new(
                id.clone(),
                AggregateVersion::initial(),
                DataClassification::Internal,
                EvidenceRole::ActionCompletion,
                EvidenceVerification::Verified {
                    verified_at: UtcTimestamp::from_unix_millis(110),
                    integrity_digest: IntegrityDigest::parse("c".repeat(64)).unwrap(),
                },
            )),
            H3Mode::Unverified => Ok(EvidenceReferenceMetadata::new(
                id.clone(),
                AggregateVersion::initial(),
                DataClassification::Internal,
                EvidenceRole::ActionCompletion,
                EvidenceVerification::Unverified,
            )),
            H3Mode::Unclassified => Ok(EvidenceReferenceMetadata::new(
                id.clone(),
                AggregateVersion::initial(),
                DataClassification::Unclassified,
                EvidenceRole::ActionCompletion,
                EvidenceVerification::Verified {
                    verified_at: UtcTimestamp::from_unix_millis(110),
                    integrity_digest: IntegrityDigest::parse("c".repeat(64)).unwrap(),
                },
            )),
        }
    }
}

type H3Service = InMemoryActionService<FixedClock, TestIds, Allow, Allow, H3Authority>;

fn h3_started_action(mode: Rc<Cell<H3Mode>>) -> (H3Service, H3Authority, ActionId) {
    let authority = H3Authority(mode);
    let mut service = H3Service::new(
        FixedClock,
        TestIds(Cell::new(0)),
        Allow,
        Allow,
        authority.clone(),
    );
    let created = service
        .create_action_request_draft(CreateActionRequestDraft {
            id: ActionRequestId::parse("request-h3-red").unwrap(),
            title: pmc_domain::BoundedText::parse("Synthetic H3 request".to_owned()).unwrap(),
            details: pmc_domain::BoundedText::parse("Synthetic H3 details".to_owned()).unwrap(),
            intended_owner: Some(StakeholderId::parse("owner-h3-red").unwrap()),
            response_due_at: None,
            intended_action_due_at: Some(UtcTimestamp::from_unix_millis(900)),
            classification: DataClassification::Internal,
            context: context("h3-red-create", "h3-red-create-correlation"),
        })
        .unwrap();
    let opened = service
        .submit_action_request(SubmitActionRequest {
            request_id: created.record.id().clone(),
            expected_version: created.record.version(),
            context: context("h3-red-submit", "h3-red-submit-correlation"),
        })
        .unwrap();
    let prepared = service
        .prepare_accept_action_request(PrepareAcceptActionRequest {
            request_id: opened.record.id().clone(),
            expected_version: opened.record.version(),
            context: context("h3-red-prepare-accept", "h3-red-prepare-accept-correlation"),
        })
        .unwrap();
    let accepted = service
        .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
            approval: WorkManagementApproval::new(
                prepared.id().clone(),
                AuditActor::HeadOfProducts,
                prepared.payload_digest().clone(),
                IdempotencyId::parse("h3-red-accept").unwrap(),
                Some(ApprovalConfirmation::Confirmed),
            )
            .unwrap(),
            context: context("h3-red-accept", "h3-red-accept-correlation"),
        })
        .unwrap();
    let started = service
        .start_action(StartAction {
            action_id: accepted.action.id().clone(),
            expected_version: accepted.action.version(),
            context: context("h3-red-start", "h3-red-start-correlation"),
        })
        .unwrap();
    (service, authority, started.record.id().clone())
}

#[test]
fn h3_denials_rehydrate_original_prepare_command_and_replay_safe_error() {
    for (index, mode) in [
        H3Mode::Missing,
        H3Mode::EvidenceNotFound,
        H3Mode::Unverified,
        H3Mode::Unclassified,
        H3Mode::Unavailable,
    ]
    .into_iter()
    .enumerate()
    {
        let mode_cell = Rc::new(Cell::new(
            if matches!(mode, H3Mode::Unavailable | H3Mode::EvidenceNotFound) {
                H3Mode::Verified
            } else {
                mode
            },
        ));
        let (mut service, authority, action_id) = h3_started_action(mode_cell.clone());
        if mode != H3Mode::Unavailable
            && mode != H3Mode::Missing
            && mode != H3Mode::EvidenceNotFound
        {
            service
                .link_action_completion_evidence(LinkActionCompletionEvidence {
                    action_id: action_id.clone(),
                    expected_version: service.action(&action_id).unwrap().version(),
                    evidence_id: EvidenceReferenceId::parse("evidence-h3-red").unwrap(),
                    context: context(
                        format!("h3-red-link-{index}"),
                        format!("h3-red-link-correlation-{index}"),
                    ),
                })
                .unwrap();
        } else if matches!(mode, H3Mode::Unavailable | H3Mode::EvidenceNotFound) {
            service
                .link_action_completion_evidence(LinkActionCompletionEvidence {
                    action_id: action_id.clone(),
                    expected_version: service.action(&action_id).unwrap().version(),
                    evidence_id: EvidenceReferenceId::parse("evidence-h3-red").unwrap(),
                    context: context(
                        "h3-red-link-unavailable",
                        "h3-red-link-unavailable-correlation",
                    ),
                })
                .unwrap();
            mode_cell.set(mode);
        }
        let command = PrepareCompleteAction {
            action_id: action_id.clone(),
            expected_version: service.action(&action_id).unwrap().version(),
            judgment: None,
            context: context(
                format!("h3-red-prepare-{index}"),
                format!("h3-red-prepare-correlation-{index}"),
            ),
        };
        let error = service
            .prepare_complete_action(command.clone())
            .unwrap_err();
        assert_eq!(
            error.code(),
            pmc_domain::error::ErrorCode::SecurityPolicyDenied
        );
        let audit_count = service.audit_events().len();
        let immediate = service
            .prepare_complete_action(PrepareCompleteAction {
                context: context(
                    format!("h3-red-prepare-{index}"),
                    format!("h3-red-immediate-correlation-{index}"),
                ),
                ..command.clone()
            })
            .unwrap_err();
        assert_eq!(immediate, error);
        assert_eq!(service.audit_events().len(), audit_count);
        let snapshot = validated(service.persistence_snapshot().unwrap());
        let expected_cause = match mode {
            H3Mode::Missing => ActionPersistenceTerminalCause::H3Denied(
                pmc_domain::actions::ActionPersistenceH3DenialCause::MissingCompletionEvidence,
            ),
            H3Mode::EvidenceNotFound => ActionPersistenceTerminalCause::H3Denied(
                pmc_domain::actions::ActionPersistenceH3DenialCause::CompletionEvidenceNotFound,
            ),
            H3Mode::Unverified => ActionPersistenceTerminalCause::H3Denied(
                pmc_domain::actions::ActionPersistenceH3DenialCause::UnverifiedCompletionEvidence,
            ),
            H3Mode::Unclassified => ActionPersistenceTerminalCause::H3Denied(
                pmc_domain::actions::ActionPersistenceH3DenialCause::UnclassifiedCompletionEvidence,
            ),
            H3Mode::Unavailable => ActionPersistenceTerminalCause::H3Denied(
                pmc_domain::actions::ActionPersistenceH3DenialCause::EvidenceUnavailable,
            ),
            H3Mode::Verified => unreachable!(),
        };
        assert!(snapshot.replay().iter().any(|capsule| matches!(
            capsule.result(),
            ActionPersistenceResult::Terminal {
                cause,
                error: persisted_error,
                prepared_disposition: PreparedDisposition::NotApplicable,
                ..
            } if cause == &expected_cause && persisted_error == &error
        )));
        let mut reopened = H3Service::rehydrate(
            FixedClock,
            TestIds(Cell::new(100)),
            Allow,
            Allow,
            authority,
            snapshot,
        );
        let audit_count = reopened.audit_events().len();
        let replayed = reopened
            .prepare_complete_action(PrepareCompleteAction {
                context: context(
                    format!("h3-red-prepare-{index}"),
                    format!("h3-red-replay-correlation-{index}"),
                ),
                ..command
            })
            .unwrap_err();
        assert_eq!(replayed, error);
        assert_eq!(reopened.audit_events().len(), audit_count);
    }
}

#[test]
fn h3_terminal_capsules_retain_complete_judgment_cancel_and_reopen_commands() {
    let (mut completion_service, _, completion_action_id) =
        h3_started_action(Rc::new(Cell::new(H3Mode::Missing)));
    let judgment = HumanJudgment::new(
        HumanJudgmentDisposition::ProceedWithDocumentedRationale,
        "Synthetic complete H3 judgment",
        DataClassification::Restricted,
    )
    .unwrap();
    completion_service
        .prepare_complete_action(PrepareCompleteAction {
            action_id: completion_action_id.clone(),
            expected_version: completion_service
                .action(&completion_action_id)
                .unwrap()
                .version(),
            judgment: Some(judgment.clone()),
            context: context("h3-complete-full", "h3-complete-full-correlation"),
        })
        .unwrap_err();
    let completion_snapshot = validated(completion_service.persistence_snapshot().unwrap());
    assert!(completion_snapshot.replay().iter().any(|capsule| matches!(
        capsule.command(),
        ActionPersistenceCommand::PrepareComplete {
            action_id,
            judgment: Some(persisted),
            ..
        } if action_id == &completion_action_id && persisted == &judgment
    )));

    let cancel_mode = Rc::new(Cell::new(H3Mode::Verified));
    let (mut cancel_service, _, cancel_action_id) = h3_started_action(cancel_mode.clone());
    cancel_service
        .link_action_completion_evidence(LinkActionCompletionEvidence {
            action_id: cancel_action_id.clone(),
            expected_version: cancel_service.action(&cancel_action_id).unwrap().version(),
            evidence_id: EvidenceReferenceId::parse("evidence-h3-red").unwrap(),
            context: context("h3-cancel-link", "h3-cancel-link-correlation"),
        })
        .unwrap();
    cancel_mode.set(H3Mode::Unavailable);
    let cancel_reason =
        pmc_domain::BoundedText::parse("Synthetic complete cancel rationale".to_owned()).unwrap();
    cancel_service
        .prepare_cancel_action(PrepareCancelAction {
            action_id: cancel_action_id.clone(),
            expected_version: cancel_service.action(&cancel_action_id).unwrap().version(),
            reason: cancel_reason.clone(),
            context: context("h3-cancel-full", "h3-cancel-full-correlation"),
        })
        .unwrap_err();
    let cancel_snapshot = validated(cancel_service.persistence_snapshot().unwrap());
    assert!(cancel_snapshot.replay().iter().any(|capsule| matches!(
        capsule.command(),
        ActionPersistenceCommand::PrepareCancel {
            action_id,
            reason,
            ..
        } if action_id == &cancel_action_id && reason == &cancel_reason
    )));

    let reopen_mode = Rc::new(Cell::new(H3Mode::Verified));
    let (mut reopen_service, _, reopen_action_id) = h3_started_action(reopen_mode.clone());
    reopen_service
        .link_action_completion_evidence(LinkActionCompletionEvidence {
            action_id: reopen_action_id.clone(),
            expected_version: reopen_service.action(&reopen_action_id).unwrap().version(),
            evidence_id: EvidenceReferenceId::parse("evidence-h3-red").unwrap(),
            context: context("h3-reopen-link", "h3-reopen-link-correlation"),
        })
        .unwrap();
    let prepared_cancel = reopen_service
        .prepare_cancel_action(PrepareCancelAction {
            action_id: reopen_action_id.clone(),
            expected_version: reopen_service.action(&reopen_action_id).unwrap().version(),
            reason: pmc_domain::BoundedText::parse("Synthetic setup cancellation".to_owned())
                .unwrap(),
            context: context(
                "h3-reopen-cancel-prepare",
                "h3-reopen-cancel-prepare-correlation",
            ),
        })
        .unwrap();
    let cancelled = reopen_service
        .approve_and_execute_cancel_action(ApproveAndExecuteCancelAction {
            approval: WorkManagementApproval::new(
                prepared_cancel.id().clone(),
                AuditActor::HeadOfProducts,
                prepared_cancel.payload_digest().clone(),
                IdempotencyId::parse("h3-reopen-cancel").unwrap(),
                Some(ApprovalConfirmation::Confirmed),
            )
            .unwrap(),
            context: context("h3-reopen-cancel", "h3-reopen-cancel-correlation"),
        })
        .unwrap();
    reopen_mode.set(H3Mode::Unavailable);
    let reopen_reason =
        pmc_domain::BoundedText::parse("Synthetic complete reopen rationale".to_owned()).unwrap();
    reopen_service
        .prepare_reopen_action(PrepareReopenAction {
            action_id: cancelled.record.id().clone(),
            expected_version: cancelled.record.version(),
            mode: ActionReopenMode::RestartCancelled,
            reason: reopen_reason.clone(),
            context: context("h3-reopen-full", "h3-reopen-full-correlation"),
        })
        .unwrap_err();
    let reopen_snapshot = validated(reopen_service.persistence_snapshot().unwrap());
    assert!(reopen_snapshot.replay().iter().any(|capsule| matches!(
        capsule.command(),
        ActionPersistenceCommand::PrepareReopen {
            action_id,
            mode: ActionReopenMode::RestartCancelled,
            reason,
            ..
        } if action_id == &reopen_action_id && reason == &reopen_reason
    )));
}

#[test]
fn restricted_completion_judgment_rehydrates_and_reopens_with_classification() {
    let (mut service, authority, action_id) =
        h3_started_action(Rc::new(Cell::new(H3Mode::Verified)));
    service
        .link_action_completion_evidence(LinkActionCompletionEvidence {
            action_id: action_id.clone(),
            expected_version: service.action(&action_id).unwrap().version(),
            evidence_id: EvidenceReferenceId::parse("evidence-h3-red").unwrap(),
            context: context("restricted-link", "restricted-link-correlation"),
        })
        .unwrap();
    let judgment = HumanJudgment::new(
        HumanJudgmentDisposition::ProceedWithDocumentedRationale,
        "Synthetic restricted completion judgment",
        DataClassification::Restricted,
    )
    .unwrap();
    let prepared = service
        .prepare_complete_action(PrepareCompleteAction {
            action_id: action_id.clone(),
            expected_version: service.action(&action_id).unwrap().version(),
            judgment: Some(judgment),
            context: context(
                "restricted-complete-prepare",
                "restricted-complete-prepare-correlation",
            ),
        })
        .unwrap();
    assert_eq!(prepared.classification(), DataClassification::Restricted);
    let completed = service
        .approve_and_execute_complete_action(ApproveAndExecuteCompleteAction {
            approval: WorkManagementApproval::new(
                prepared.id().clone(),
                AuditActor::HeadOfProducts,
                prepared.payload_digest().clone(),
                IdempotencyId::parse("restricted-complete").unwrap(),
                Some(ApprovalConfirmation::Confirmed),
            )
            .unwrap(),
            context: context("restricted-complete", "restricted-complete-correlation"),
        })
        .unwrap();
    let prepared_reopen = service
        .prepare_reopen_action(PrepareReopenAction {
            action_id: completed.record.id().clone(),
            expected_version: completed.record.version(),
            mode: ActionReopenMode::ReopenCompleted,
            reason: pmc_domain::BoundedText::parse("Synthetic restricted follow-up".to_owned())
                .unwrap(),
            context: context(
                "restricted-reopen-prepare",
                "restricted-reopen-prepare-correlation",
            ),
        })
        .unwrap();
    let reopen_approval = WorkManagementApproval::new(
        prepared_reopen.id().clone(),
        AuditActor::HeadOfProducts,
        prepared_reopen.payload_digest().clone(),
        IdempotencyId::parse("restricted-reopen").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    let snapshot = validated(service.persistence_snapshot().unwrap());
    let mut reopened = H3Service::rehydrate(
        FixedClock,
        TestIds(Cell::new(100)),
        Allow,
        Allow,
        authority,
        snapshot,
    );
    let reopened_action = reopened
        .approve_and_execute_reopen_action(ApproveAndExecuteReopenAction {
            approval: reopen_approval,
            context: context("restricted-reopen", "restricted-reopen-correlation"),
        })
        .unwrap();
    assert_eq!(
        reopened_action.record.classification(),
        DataClassification::Restricted
    );
}

#[test]
fn h3_terminal_replay_uses_persisted_error_without_current_evidence_authority() {
    let mode = Rc::new(Cell::new(H3Mode::Verified));
    let (mut service, authority, action_id) = h3_started_action(mode.clone());
    service
        .link_action_completion_evidence(LinkActionCompletionEvidence {
            action_id: action_id.clone(),
            expected_version: service.action(&action_id).unwrap().version(),
            evidence_id: EvidenceReferenceId::parse("evidence-h3-red").unwrap(),
            context: context("h3-replay-link", "h3-replay-link-correlation"),
        })
        .unwrap();
    mode.set(H3Mode::EvidenceNotFound);
    let command = PrepareCompleteAction {
        action_id: action_id.clone(),
        expected_version: service.action(&action_id).unwrap().version(),
        judgment: None,
        context: context("h3-replay-prepare", "h3-replay-prepare-correlation"),
    };
    let original = service
        .prepare_complete_action(command.clone())
        .unwrap_err();
    let snapshot = validated(service.persistence_snapshot().unwrap());

    // The authority is now unavailable. A same-id replay must use the persisted
    // terminal DomainError and original correlation, without resolving evidence.
    mode.set(H3Mode::Unavailable);
    let mut reopened = H3Service::rehydrate(
        FixedClock,
        TestIds(Cell::new(100)),
        Allow,
        Allow,
        authority,
        snapshot,
    );
    let replayed = reopened
        .prepare_complete_action(PrepareCompleteAction {
            context: context("h3-replay-prepare", "h3-replay-new-correlation"),
            ..command.clone()
        })
        .unwrap_err();
    assert_eq!(replayed, original);

    let altered_judgment = HumanJudgment::new(
        HumanJudgmentDisposition::ProceedWithDocumentedRationale,
        "Synthetic altered H3 judgment",
        DataClassification::Internal,
    )
    .unwrap();
    let conflict = reopened
        .prepare_complete_action(PrepareCompleteAction {
            judgment: Some(altered_judgment),
            context: context("h3-replay-prepare", "h3-replay-altered-correlation"),
            ..command
        })
        .unwrap_err();
    assert_eq!(
        conflict.code(),
        pmc_domain::error::ErrorCode::DomainIdempotencyConflict
    );
}

#[test]
fn h3_evidence_not_found_cancel_and_reopen_rehydrate_and_replay_exactly() {
    let cancel_mode = Rc::new(Cell::new(H3Mode::Verified));
    let (mut cancel_service, cancel_authority, cancel_action_id) =
        h3_started_action(cancel_mode.clone());
    cancel_service
        .link_action_completion_evidence(LinkActionCompletionEvidence {
            action_id: cancel_action_id.clone(),
            expected_version: cancel_service.action(&cancel_action_id).unwrap().version(),
            evidence_id: EvidenceReferenceId::parse("evidence-h3-red").unwrap(),
            context: context(
                "h3-not-found-cancel-link",
                "h3-not-found-cancel-link-correlation",
            ),
        })
        .unwrap();
    cancel_mode.set(H3Mode::EvidenceNotFound);
    let cancel_command = PrepareCancelAction {
        action_id: cancel_action_id.clone(),
        expected_version: cancel_service.action(&cancel_action_id).unwrap().version(),
        reason: pmc_domain::BoundedText::parse("Synthetic not-found cancel rationale".to_owned())
            .unwrap(),
        context: context("h3-not-found-cancel", "h3-not-found-cancel-correlation"),
    };
    let cancel_error = cancel_service
        .prepare_cancel_action(cancel_command.clone())
        .unwrap_err();
    let cancel_audits = cancel_service.audit_events().len();
    let cancel_snapshot = validated(cancel_service.persistence_snapshot().unwrap());
    let mut cancel_reopened = H3Service::rehydrate(
        FixedClock,
        TestIds(Cell::new(100)),
        Allow,
        Allow,
        cancel_authority,
        cancel_snapshot,
    );
    assert_eq!(cancel_reopened.audit_events().len(), cancel_audits);
    assert_eq!(
        cancel_reopened
            .prepare_cancel_action(PrepareCancelAction {
                context: context("h3-not-found-cancel", "h3-not-found-cancel-replay"),
                ..cancel_command
            })
            .unwrap_err(),
        cancel_error
    );
    assert_eq!(cancel_reopened.audit_events().len(), cancel_audits);

    let reopen_mode = Rc::new(Cell::new(H3Mode::Verified));
    let (mut reopen_service, reopen_authority, reopen_action_id) =
        h3_started_action(reopen_mode.clone());
    reopen_service
        .link_action_completion_evidence(LinkActionCompletionEvidence {
            action_id: reopen_action_id.clone(),
            expected_version: reopen_service.action(&reopen_action_id).unwrap().version(),
            evidence_id: EvidenceReferenceId::parse("evidence-h3-red").unwrap(),
            context: context(
                "h3-not-found-reopen-link",
                "h3-not-found-reopen-link-correlation",
            ),
        })
        .unwrap();
    let prepared_cancel = reopen_service
        .prepare_cancel_action(PrepareCancelAction {
            action_id: reopen_action_id.clone(),
            expected_version: reopen_service.action(&reopen_action_id).unwrap().version(),
            reason: pmc_domain::BoundedText::parse("Synthetic setup cancellation".to_owned())
                .unwrap(),
            context: context(
                "h3-not-found-reopen-cancel",
                "h3-not-found-reopen-cancel-correlation",
            ),
        })
        .unwrap();
    let cancelled = reopen_service
        .approve_and_execute_cancel_action(ApproveAndExecuteCancelAction {
            approval: WorkManagementApproval::new(
                prepared_cancel.id().clone(),
                AuditActor::HeadOfProducts,
                prepared_cancel.payload_digest().clone(),
                IdempotencyId::parse("h3-not-found-reopen-execute").unwrap(),
                Some(ApprovalConfirmation::Confirmed),
            )
            .unwrap(),
            context: context(
                "h3-not-found-reopen-execute",
                "h3-not-found-reopen-execute-correlation",
            ),
        })
        .unwrap();
    reopen_mode.set(H3Mode::EvidenceNotFound);
    let reopen_command = PrepareReopenAction {
        action_id: reopen_action_id.clone(),
        expected_version: cancelled.record.version(),
        mode: ActionReopenMode::RestartCancelled,
        reason: pmc_domain::BoundedText::parse("Synthetic not-found reopen rationale".to_owned())
            .unwrap(),
        context: context("h3-not-found-reopen", "h3-not-found-reopen-correlation"),
    };
    let reopen_error = reopen_service
        .prepare_reopen_action(reopen_command.clone())
        .unwrap_err();
    let reopen_audits = reopen_service.audit_events().len();
    let reopen_snapshot = validated(reopen_service.persistence_snapshot().unwrap());
    let mut reopen_rehydrated = H3Service::rehydrate(
        FixedClock,
        TestIds(Cell::new(100)),
        Allow,
        Allow,
        reopen_authority,
        reopen_snapshot,
    );
    assert_eq!(reopen_rehydrated.audit_events().len(), reopen_audits);
    assert_eq!(
        reopen_rehydrated
            .prepare_reopen_action(PrepareReopenAction {
                context: context("h3-not-found-reopen", "h3-not-found-reopen-replay"),
                ..reopen_command
            })
            .unwrap_err(),
        reopen_error
    );
    assert_eq!(reopen_rehydrated.audit_events().len(), reopen_audits);
}

#[test]
fn h3_cancel_and_reopen_idempotency_conflicts_are_detected_after_rehydration() {
    let mode = Rc::new(Cell::new(H3Mode::Verified));
    let (mut cancel_service, authority, action_id) = h3_started_action(mode.clone());
    cancel_service
        .link_action_completion_evidence(LinkActionCompletionEvidence {
            action_id: action_id.clone(),
            expected_version: cancel_service.action(&action_id).unwrap().version(),
            evidence_id: EvidenceReferenceId::parse("evidence-h3-red").unwrap(),
            context: context(
                "h3-conflict-cancel-link",
                "h3-conflict-cancel-link-correlation",
            ),
        })
        .unwrap();
    mode.set(H3Mode::EvidenceNotFound);
    let cancel_command = PrepareCancelAction {
        action_id: action_id.clone(),
        expected_version: cancel_service.action(&action_id).unwrap().version(),
        reason: pmc_domain::BoundedText::parse("Synthetic original cancel rationale".to_owned())
            .unwrap(),
        context: context("h3-conflict-cancel", "h3-conflict-cancel-correlation"),
    };
    let _ = cancel_service.prepare_cancel_action(cancel_command.clone());
    let snapshot = validated(cancel_service.persistence_snapshot().unwrap());
    let mut reopened = H3Service::rehydrate(
        FixedClock,
        TestIds(Cell::new(100)),
        Allow,
        Allow,
        authority,
        snapshot,
    );
    let audit_count = reopened.audit_events().len();
    let altered = reopened
        .prepare_cancel_action(PrepareCancelAction {
            reason: pmc_domain::BoundedText::parse("Synthetic altered cancel rationale".to_owned())
                .unwrap(),
            context: context("h3-conflict-cancel", "h3-conflict-cancel-altered"),
            ..cancel_command
        })
        .unwrap_err();
    assert_eq!(
        altered.code(),
        pmc_domain::error::ErrorCode::DomainIdempotencyConflict
    );
    assert_eq!(reopened.audit_events().len(), audit_count);

    let mode = Rc::new(Cell::new(H3Mode::Verified));
    let (mut reopen_service, authority, action_id) = h3_started_action(mode.clone());
    reopen_service
        .link_action_completion_evidence(LinkActionCompletionEvidence {
            action_id: action_id.clone(),
            expected_version: reopen_service.action(&action_id).unwrap().version(),
            evidence_id: EvidenceReferenceId::parse("evidence-h3-red").unwrap(),
            context: context(
                "h3-conflict-reopen-link",
                "h3-conflict-reopen-link-correlation",
            ),
        })
        .unwrap();
    let prepared_cancel = reopen_service
        .prepare_cancel_action(PrepareCancelAction {
            action_id: action_id.clone(),
            expected_version: reopen_service.action(&action_id).unwrap().version(),
            reason: pmc_domain::BoundedText::parse("Synthetic conflict setup".to_owned()).unwrap(),
            context: context(
                "h3-conflict-reopen-cancel",
                "h3-conflict-reopen-cancel-correlation",
            ),
        })
        .unwrap();
    let cancelled = reopen_service
        .approve_and_execute_cancel_action(ApproveAndExecuteCancelAction {
            approval: WorkManagementApproval::new(
                prepared_cancel.id().clone(),
                AuditActor::HeadOfProducts,
                prepared_cancel.payload_digest().clone(),
                IdempotencyId::parse("h3-conflict-reopen-execute").unwrap(),
                Some(ApprovalConfirmation::Confirmed),
            )
            .unwrap(),
            context: context(
                "h3-conflict-reopen-execute",
                "h3-conflict-reopen-execute-correlation",
            ),
        })
        .unwrap();
    mode.set(H3Mode::EvidenceNotFound);
    let reopen_command = PrepareReopenAction {
        action_id: action_id.clone(),
        expected_version: cancelled.record.version(),
        mode: ActionReopenMode::RestartCancelled,
        reason: pmc_domain::BoundedText::parse("Synthetic original reopen rationale".to_owned())
            .unwrap(),
        context: context("h3-conflict-reopen", "h3-conflict-reopen-correlation"),
    };
    let _ = reopen_service.prepare_reopen_action(reopen_command.clone());
    let snapshot = validated(reopen_service.persistence_snapshot().unwrap());
    let mut reopened = H3Service::rehydrate(
        FixedClock,
        TestIds(Cell::new(100)),
        Allow,
        Allow,
        authority,
        snapshot,
    );
    let audit_count = reopened.audit_events().len();
    let altered = reopened
        .prepare_reopen_action(PrepareReopenAction {
            mode: ActionReopenMode::ReopenCompleted,
            reason: pmc_domain::BoundedText::parse("Synthetic altered reopen rationale".to_owned())
                .unwrap(),
            context: context("h3-conflict-reopen", "h3-conflict-reopen-altered"),
            ..reopen_command
        })
        .unwrap_err();
    assert_eq!(
        altered.code(),
        pmc_domain::error::ErrorCode::DomainIdempotencyConflict
    );
    assert_eq!(reopened.audit_events().len(), audit_count);
}
