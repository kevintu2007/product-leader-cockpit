use pmc_domain::audit::AuditActor;
use pmc_domain::classification::DataClassification;
use pmc_domain::identity::*;
use pmc_domain::risks::*;
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::work_management::{
    ApprovalConfirmation, RiskResponseType, WorkManagementApproval, WorkManagementOperation,
};
use std::cell::Cell;
use std::rc::Rc;

use pmc_domain::audit::{
    AuditApprovalOutcome, AuditEffectScope, AuditExecutionOutcome, AuditPolicyOutcome,
};
use pmc_domain::error::{DomainError, ErrorCode, SafeErrorExtension, SafeParamValue};

#[derive(Clone)]
struct TestClock(Rc<Cell<i64>>);
impl Clock for TestClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(self.0.get())
    }
}
struct Ids(u64);
impl RiskServiceIdSource for Ids {
    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<PreparedIntentId, pmc_domain::DomainValueError> {
        self.0 += 1;
        PreparedIntentId::parse(format!("prepared-{}", self.0))
    }
    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<ApprovalReceiptId, pmc_domain::DomainValueError> {
        self.0 += 1;
        ApprovalReceiptId::parse(format!("receipt-{}", self.0))
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, pmc_domain::DomainValueError> {
        self.0 += 1;
        AuditEventId::parse(format!("audit-{}", self.0))
    }
}
#[derive(Clone, Copy)]
struct Gate;
impl pmc_domain::work_management::ApprovalAuthorizationPort for Gate {
    fn authorize(&self, a: AuditActor) -> bool {
        a == AuditActor::HeadOfProducts
    }
}
impl RiskExecutionPolicyPort for Gate {
    fn current_policy(&self, _: &WorkManagementOperation) -> RiskExecutionPolicy {
        RiskExecutionPolicy::Allowed
    }
}
fn txt<const N: usize>(s: &str) -> pmc_domain::BoundedText<N> {
    pmc_domain::BoundedText::parse(s.to_owned()).unwrap()
}
fn ctx(s: &str) -> RiskOperationContext {
    RiskOperationContext {
        idempotency_id: IdempotencyId::parse(s).unwrap(),
        correlation_id: CorrelationId::parse(format!("corr-{s}")).unwrap(),
    }
}
fn service(
) -> InMemoryRiskService<TestClock, Ids, Gate, Gate, AllowRiskEvidence, RecordedRiskClassification>
{
    InMemoryRiskService::new(
        TestClock(Rc::new(Cell::new(100))),
        Ids(0),
        Gate,
        Gate,
        AllowRiskEvidence,
        RecordedRiskClassification,
    )
}
fn create(s: &str) -> CreateRisk {
    CreateRisk {
        id: RiskId::parse("risk-1").unwrap(),
        title: txt("Synthetic risk"),
        details: txt("Synthetic details"),
        classification: DataClassification::Internal,
        context: ctx(s),
    }
}

#[test]
fn create_and_response_are_typed_and_versioned() {
    let mut s = service();
    let out = s.create_risk(create("create")).unwrap();
    assert_eq!(
        out.record.state(),
        pmc_domain::work_management::RiskState::Open
    );
    let out = s
        .update_risk_response(UpdateRiskResponse {
            risk_id: out.record.id().clone(),
            expected_version: out.record.version(),
            response: RiskResponseType::Accept,
            owner: Some(StakeholderId::parse("owner").unwrap()),
            rationale: Some(txt("accepted rationale")),
            residual_exposure: Some(txt("medium")),
            next_review_at: Some(UtcTimestamp::from_unix_millis(200)),
            context: ctx("update"),
        })
        .unwrap();
    assert!(!out.record.in_exception_queue());
    assert_eq!(out.record.version().get(), 2);
}
#[test]
fn occurrence_atomically_creates_one_linked_issue() {
    let mut s = service();
    let r = s.create_risk(create("create")).unwrap().record;
    let p = s
        .prepare_record_risk_occurrence(PrepareRecordRiskOccurrence {
            risk_id: r.id().clone(),
            expected_version: r.version(),
            issue_id: IssueId::parse("issue-1").unwrap(),
            context: ctx("prepare"),
        })
        .unwrap();
    let a = WorkManagementApproval::new(
        p.id().clone(),
        AuditActor::HeadOfProducts,
        p.payload_digest().clone(),
        IdempotencyId::parse("execute").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    let out = s
        .approve_and_execute_record_risk_occurrence(ApproveAndExecuteRecordRiskOccurrence {
            approval: a,
            context: ctx("execute"),
        })
        .unwrap();
    assert_eq!(
        out.risk.state(),
        pmc_domain::work_management::RiskState::Occurred
    );
    assert_eq!(out.issue.source_risk_id(), Some(r.id()));
    assert!(s.issue(out.issue.id()).is_some());
    let snapshot = RiskPersistenceSnapshot::try_new(
        vec![out.risk.clone()],
        vec![out.issue.clone()],
        s.risk_issue_links().to_vec(),
    )
    .unwrap();
    assert_eq!(snapshot.occurrence_issues(), &[out.issue]);
    assert_eq!(snapshot.links().len(), 1);
}

#[test]
fn closed_risk_rehydration_accepts_only_the_exact_created_open_terminal_shape() {
    let version = AggregateVersion::initial().next().unwrap();
    let closed = RiskRecord::from_persisted_closed_from_created_open(
        RiskId::parse("risk-closed-rehydration").unwrap(),
        txt("Synthetic closed risk"),
        txt("Synthetic closed-risk details"),
        DataClassification::Internal,
        version,
    )
    .unwrap();

    assert_eq!(
        closed.state(),
        pmc_domain::work_management::RiskState::Closed
    );
    assert!(!closed.in_exception_queue());
    assert_eq!(closed.response(), None);
    assert_eq!(closed.version(), version);

    assert!(RiskRecord::from_persisted_closed_from_created_open(
        RiskId::parse("risk-unclassified-rehydration").unwrap(),
        txt("Synthetic closed risk"),
        txt("Synthetic closed-risk details"),
        DataClassification::Unclassified,
        version,
    )
    .is_err());
    assert!(RiskRecord::from_persisted_closed_from_created_open(
        RiskId::parse("risk-wrong-version-rehydration").unwrap(),
        txt("Synthetic closed risk"),
        txt("Synthetic closed-risk details"),
        DataClassification::Internal,
        AggregateVersion::initial(),
    )
    .is_err());
}

#[test]
fn occurrence_snapshot_requires_exactly_one_classification_matched_linked_issue() {
    let mut s = service();
    let risk = s.create_risk(create("create-cardinality")).unwrap().record;
    let prepared = s
        .prepare_record_risk_occurrence(PrepareRecordRiskOccurrence {
            risk_id: risk.id().clone(),
            expected_version: risk.version(),
            issue_id: IssueId::parse("issue-cardinality").unwrap(),
            context: ctx("prepare-cardinality"),
        })
        .unwrap();
    let approval = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("execute-cardinality").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    let occurred = s
        .approve_and_execute_record_risk_occurrence(ApproveAndExecuteRecordRiskOccurrence {
            approval,
            context: ctx("execute-cardinality"),
        })
        .unwrap();

    assert!(RiskPersistenceSnapshot::try_new(vec![occurred.risk.clone()], vec![], vec![]).is_err());

    let mismatched_link = RiskIssueLink::from_persisted(
        occurred.risk.id().clone(),
        occurred.issue.id().clone(),
        occurred.risk.version(),
        occurred.issue.version(),
        DataClassification::Confidential,
    )
    .unwrap();
    assert!(RiskPersistenceSnapshot::try_new(
        vec![occurred.risk.clone()],
        vec![occurred.issue.clone()],
        vec![mismatched_link],
    )
    .is_err());

    let second_issue = pmc_domain::issues::IssueRecord::from_persisted_created_from_risk(
        IssueId::parse("issue-cardinality-second").unwrap(),
        risk.id().clone(),
        pmc_domain::issues::IssueTitle::parse("Second synthetic occurrence issue").unwrap(),
        pmc_domain::issues::IssueDetails::parse("Synthetic only; no organizational data.").unwrap(),
        DataClassification::Internal,
        AggregateVersion::initial(),
    )
    .unwrap();
    let first_link = RiskIssueLink::from_persisted(
        occurred.risk.id().clone(),
        occurred.issue.id().clone(),
        occurred.risk.version(),
        occurred.issue.version(),
        DataClassification::Internal,
    )
    .unwrap();
    let second_link = RiskIssueLink::from_persisted(
        occurred.risk.id().clone(),
        second_issue.id().clone(),
        occurred.risk.version(),
        second_issue.version(),
        DataClassification::Internal,
    )
    .unwrap();
    assert!(RiskPersistenceSnapshot::try_new(
        vec![occurred.risk],
        vec![occurred.issue, second_issue],
        vec![first_link, second_link],
    )
    .is_err());
}

#[test]
fn every_response_type_is_an_attribute_and_accept_transfer_require_complete_fields() {
    for (idx, response) in [
        RiskResponseType::Mitigate,
        RiskResponseType::Accept,
        RiskResponseType::Transfer,
        RiskResponseType::Avoid,
    ]
    .into_iter()
    .enumerate()
    {
        let mut service = service();
        let risk = service
            .create_risk(create(&format!("create-{idx}")))
            .unwrap()
            .record;
        let complete = matches!(
            response,
            RiskResponseType::Accept | RiskResponseType::Transfer
        );
        let result = service
            .update_risk_response(UpdateRiskResponse {
                risk_id: risk.id().clone(),
                expected_version: risk.version(),
                response,
                owner: complete.then(|| StakeholderId::parse("owner").unwrap()),
                rationale: complete.then(|| txt("rationale")),
                residual_exposure: complete.then(|| txt("medium")),
                next_review_at: complete.then(|| UtcTimestamp::from_unix_millis(200)),
                context: ctx(&format!("update-{idx}")),
            })
            .unwrap();
        assert_eq!(result.record.response(), Some(response));
        assert_eq!(result.record.in_exception_queue(), !complete);
    }
}

#[test]
fn each_reentry_trigger_is_pure_and_preserves_version_and_audit() {
    let triggers = [
        (UtcTimestamp::from_unix_millis(200), false, false),
        (UtcTimestamp::from_unix_millis(100), true, false),
        (UtcTimestamp::from_unix_millis(100), false, true),
    ];
    for (idx, (now, increased, invalid)) in triggers.into_iter().enumerate() {
        let mut service = service();
        let risk = service
            .create_risk(create(&format!("r-{idx}")))
            .unwrap()
            .record;
        let risk = service
            .update_risk_response(UpdateRiskResponse {
                risk_id: risk.id().clone(),
                expected_version: risk.version(),
                response: RiskResponseType::Accept,
                owner: Some(StakeholderId::parse("owner").unwrap()),
                rationale: Some(txt("why")),
                residual_exposure: Some(txt("medium")),
                next_review_at: Some(UtcTimestamp::from_unix_millis(200)),
                context: ctx(&format!("u-{idx}")),
            })
            .unwrap()
            .record;
        let before = (risk.version(), service.audit_events().len());
        assert!(service
            .risk_reenters_queue(risk.id(), now, increased, invalid)
            .unwrap());
        let after = service.risk(risk.id()).unwrap();
        assert_eq!((after.version(), service.audit_events().len()), before);
    }
}

#[test]
fn close_requires_open_risk_and_rationale_and_is_terminal() {
    let mut service = service();
    let risk = service.create_risk(create("close-create")).unwrap().record;
    let prepared = service
        .prepare_close_risk(PrepareCloseRisk {
            risk_id: risk.id().clone(),
            expected_version: risk.version(),
            rationale: txt("closed by owner"),
            context: ctx("close-prepare"),
        })
        .unwrap();
    let approval = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("close-execute").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    let outcome = service
        .approve_and_execute_close_risk(ApproveAndExecuteCloseRisk {
            approval,
            context: ctx("close-execute"),
        })
        .unwrap();
    assert_eq!(
        outcome.record.state(),
        pmc_domain::work_management::RiskState::Closed
    );
    assert!(!outcome.record.in_exception_queue());
    assert!(service
        .prepare_close_risk(PrepareCloseRisk {
            risk_id: risk.id().clone(),
            expected_version: outcome.record.version(),
            rationale: txt("again"),
            context: ctx("close-again")
        })
        .is_err());
}

#[derive(Clone)]
struct MutableGate {
    authorized: Rc<Cell<bool>>,
    allowed: Rc<Cell<bool>>,
}
impl pmc_domain::work_management::ApprovalAuthorizationPort for MutableGate {
    fn authorize(&self, _: AuditActor) -> bool {
        self.authorized.get()
    }
}
impl RiskExecutionPolicyPort for MutableGate {
    fn current_policy(&self, _: &WorkManagementOperation) -> RiskExecutionPolicy {
        if self.allowed.get() {
            RiskExecutionPolicy::Allowed
        } else {
            RiskExecutionPolicy::Denied
        }
    }
}
#[derive(Clone)]
struct MutableEvidence(Rc<Cell<i8>>);
impl RiskEvidenceAuthorityPort for MutableEvidence {
    fn linked_evidence_is_current(&self, _: &RiskId) -> Result<bool, RiskEvidenceAuthorityError> {
        match self.0.get() {
            -1 => Err(RiskEvidenceAuthorityError::Unavailable),
            0 => Ok(false),
            _ => Ok(true),
        }
    }
}
#[derive(Clone)]
struct MutableClassification(Rc<Cell<DataClassification>>);
impl RiskClassificationAuthorityPort for MutableClassification {
    fn current_classification(
        &self,
        _: &RiskId,
        _: DataClassification,
    ) -> Result<DataClassification, RiskEvidenceAuthorityError> {
        Ok(self.0.get())
    }
}
struct FailableIds {
    next: u64,
    fail_audit: Rc<Cell<bool>>,
}
impl RiskServiceIdSource for FailableIds {
    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<PreparedIntentId, pmc_domain::DomainValueError> {
        self.next += 1;
        PreparedIntentId::parse(format!("prepared-{}", self.next))
    }
    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<ApprovalReceiptId, pmc_domain::DomainValueError> {
        self.next += 1;
        ApprovalReceiptId::parse(format!("receipt-{}", self.next))
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, pmc_domain::DomainValueError> {
        if self.fail_audit.replace(false) {
            return AuditEventId::parse("bad id");
        }
        self.next += 1;
        AuditEventId::parse(format!("audit-{}", self.next))
    }
}
type ConfigService = InMemoryRiskService<
    TestClock,
    FailableIds,
    MutableGate,
    MutableGate,
    MutableEvidence,
    MutableClassification,
>;
struct Harness {
    service: ConfigService,
    time: Rc<Cell<i64>>,
    authorized: Rc<Cell<bool>>,
    allowed: Rc<Cell<bool>>,
    evidence: Rc<Cell<i8>>,
    classification: Rc<Cell<DataClassification>>,
    fail_audit: Rc<Cell<bool>>,
}
fn harness() -> Harness {
    let time = Rc::new(Cell::new(100));
    let authorized = Rc::new(Cell::new(true));
    let allowed = Rc::new(Cell::new(true));
    let evidence = Rc::new(Cell::new(1));
    let fail_audit = Rc::new(Cell::new(false));
    let classification = Rc::new(Cell::new(DataClassification::Internal));
    let gate = MutableGate {
        authorized: authorized.clone(),
        allowed: allowed.clone(),
    };
    Harness {
        service: InMemoryRiskService::new(
            TestClock(time.clone()),
            FailableIds {
                next: 0,
                fail_audit: fail_audit.clone(),
            },
            gate.clone(),
            gate,
            MutableEvidence(evidence.clone()),
            MutableClassification(classification.clone()),
        ),
        time,
        authorized,
        allowed,
        evidence,
        classification,
        fail_audit,
    }
}
fn prepared_occurrence(
    h: &mut Harness,
    prepare_key: &str,
    issue: &str,
) -> (
    RiskRecord,
    pmc_domain::work_management::WorkManagementPreparedIntent,
) {
    let risk = h
        .service
        .create_risk(create(&format!("create-{prepare_key}")))
        .unwrap()
        .record;
    let prepared = h
        .service
        .prepare_record_risk_occurrence(PrepareRecordRiskOccurrence {
            risk_id: risk.id().clone(),
            expected_version: risk.version(),
            issue_id: IssueId::parse(issue).unwrap(),
            context: ctx(prepare_key),
        })
        .unwrap();
    (risk, prepared)
}
fn approval(
    p: &pmc_domain::work_management::WorkManagementPreparedIntent,
    key: &str,
) -> WorkManagementApproval {
    WorkManagementApproval::new(
        p.id().clone(),
        AuditActor::HeadOfProducts,
        p.payload_digest().clone(),
        IdempotencyId::parse(key).unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap()
}
fn execute_occurrence(
    h: &mut Harness,
    p: &pmc_domain::work_management::WorkManagementPreparedIntent,
    key: &str,
) -> Result<OccurredRiskOutcome, Box<DomainError>> {
    h.service
        .approve_and_execute_record_risk_occurrence(ApproveAndExecuteRecordRiskOccurrence {
            approval: approval(p, key),
            context: ctx(key),
        })
        .map_err(Box::new)
}

#[test]
fn accept_and_transfer_each_reject_every_incomplete_required_field_set() {
    for response in [RiskResponseType::Accept, RiskResponseType::Transfer] {
        for missing in 0..4 {
            let mut s = service();
            let r = s
                .create_risk(create(&format!("c-{missing}-{response:?}")))
                .unwrap()
                .record;
            let mut owner = Some(StakeholderId::parse("owner").unwrap());
            let mut rationale = Some(txt("why"));
            let mut residual = Some(txt("medium"));
            let mut review = Some(UtcTimestamp::from_unix_millis(200));
            match missing {
                0 => owner = None,
                1 => rationale = None,
                2 => residual = None,
                _ => review = None,
            }
            let error = s
                .update_risk_response(UpdateRiskResponse {
                    risk_id: r.id().clone(),
                    expected_version: r.version(),
                    response,
                    owner,
                    rationale,
                    residual_exposure: residual,
                    next_review_at: review,
                    context: ctx(&format!("u-{missing}-{response:?}")),
                })
                .unwrap_err();
            assert_eq!(error.code(), ErrorCode::ValidationInvalidField);
            assert_eq!(s.risk(r.id()).unwrap().version(), r.version());
        }
    }
}

#[test]
fn evidence_staleness_is_the_fourth_pure_reentry_trigger() {
    let mut h = harness();
    let risk = h
        .service
        .create_risk(create("evidence-create"))
        .unwrap()
        .record;
    let risk = h
        .service
        .update_risk_response(UpdateRiskResponse {
            risk_id: risk.id().clone(),
            expected_version: risk.version(),
            response: RiskResponseType::Accept,
            owner: Some(StakeholderId::parse("owner").unwrap()),
            rationale: Some(txt("why")),
            residual_exposure: Some(txt("medium")),
            next_review_at: Some(UtcTimestamp::from_unix_millis(500)),
            context: ctx("evidence-update"),
        })
        .unwrap()
        .record;
    h.evidence.set(0);
    let before = (risk.version(), h.service.audit_events().len());
    assert!(h
        .service
        .risk_reenters_queue(risk.id(), UtcTimestamp::from_unix_millis(100), false, false)
        .unwrap());
    assert_eq!(
        (
            h.service.risk(risk.id()).unwrap().version(),
            h.service.audit_events().len()
        ),
        before
    );
}

#[test]
fn occurrence_link_is_queryable_classified_audited_and_exactly_replayed() {
    let mut h = harness();
    let (risk, p) = prepared_occurrence(&mut h, "link-prepare", "issue-link");
    let out = execute_occurrence(&mut h, &p, "link-execute").unwrap();
    let link = h
        .service
        .risk_issue_link(risk.id(), out.issue.id())
        .unwrap();
    assert_eq!(link.classification(), DataClassification::Internal);
    assert_eq!(link.risk_version(), out.risk.version());
    assert_eq!(out.audit_events.len(), 3);
    let audit_count = h.service.audit_events().len();
    assert_eq!(execute_occurrence(&mut h, &p, "link-execute").unwrap(), out);
    assert_eq!(h.service.audit_events().len(), audit_count);
}

#[test]
fn prepare_and_execute_replay_collisions_and_discard_are_deterministic() {
    let mut h = harness();
    let risk = h.service.create_risk(create("idem-create")).unwrap().record;
    let command = PrepareRecordRiskOccurrence {
        risk_id: risk.id().clone(),
        expected_version: risk.version(),
        issue_id: IssueId::parse("issue-a").unwrap(),
        context: ctx("idem-prepare"),
    };
    let p = h
        .service
        .prepare_record_risk_occurrence(command.clone())
        .unwrap();
    assert_eq!(
        h.service.prepare_record_risk_occurrence(command).unwrap(),
        p
    );
    let collision = h
        .service
        .prepare_record_risk_occurrence(PrepareRecordRiskOccurrence {
            risk_id: risk.id().clone(),
            expected_version: risk.version(),
            issue_id: IssueId::parse("issue-b").unwrap(),
            context: ctx("idem-prepare"),
        })
        .unwrap_err();
    assert_eq!(collision.code(), ErrorCode::DomainIdempotencyConflict);
    assert!(h.service.discard_prepared_intent(p.id()));
    let restarted = h
        .service
        .prepare_record_risk_occurrence(PrepareRecordRiskOccurrence {
            risk_id: risk.id().clone(),
            expected_version: risk.version(),
            issue_id: IssueId::parse("issue-a").unwrap(),
            context: ctx("idem-prepare"),
        })
        .unwrap();
    assert_ne!(restarted.id(), p.id());
    let out = execute_occurrence(&mut h, &restarted, "idem-execute").unwrap();
    assert_eq!(
        execute_occurrence(&mut h, &restarted, "idem-execute").unwrap(),
        out
    );
    let different = WorkManagementApproval::new(
        PreparedIntentId::parse("prepared-other").unwrap(),
        AuditActor::HeadOfProducts,
        restarted.payload_digest().clone(),
        IdempotencyId::parse("idem-execute").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    let error = h
        .service
        .approve_and_execute_record_risk_occurrence(ApproveAndExecuteRecordRiskOccurrence {
            approval: different,
            context: ctx("idem-execute"),
        })
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::DomainIdempotencyConflict);
}

#[test]
fn reconstructed_service_has_no_pending_prepared_authority() {
    let mut original = harness();
    let (_, prepared) = prepared_occurrence(&mut original, "restart-prepare", "issue-restart");
    let mut reconstructed = harness();
    let error = execute_occurrence(&mut reconstructed, &prepared, "restart-execute").unwrap_err();
    assert_eq!(error.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
    assert!(reconstructed.service.risk_issue_links().is_empty());
}

#[test]
fn post_start_execution_denial_replays_the_original_safe_error_without_new_effects() {
    let mut h = harness();
    let (risk, prepared) = prepared_occurrence(&mut h, "terminal-replay-prepare", "issue-terminal");
    h.allowed.set(false);

    let first = execute_occurrence(&mut h, &prepared, "terminal-replay-execute").unwrap_err();
    let audits_after_first = h.service.audit_events().len();

    h.allowed.set(true);
    let replay = execute_occurrence(&mut h, &prepared, "terminal-replay-execute").unwrap_err();

    assert_eq!(replay, first);
    assert_eq!(h.service.audit_events().len(), audits_after_first);
    assert_eq!(
        h.service.risk(risk.id()).unwrap().state(),
        pmc_domain::work_management::RiskState::Open
    );
    assert!(h
        .service
        .issue(&IssueId::parse("issue-terminal").unwrap())
        .is_none());

    let new_execution = execute_occurrence(
        &mut h,
        &prepared,
        "terminal-replay-new-execution-identifier",
    )
    .unwrap_err();
    assert_eq!(
        new_execution.code(),
        ErrorCode::SecurityPreviewExpiredOrChanged
    );
    assert!(h
        .service
        .issue(&IssueId::parse("issue-terminal").unwrap())
        .is_none());
}

#[test]
fn pre_execution_digest_correction_is_not_durable_terminal_history() {
    let mut h = harness();
    let (_, prepared) = prepared_occurrence(&mut h, "correction-prepare", "issue-correction");
    let invalid = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        pmc_domain::work_management::WorkManagementPayloadDigest::from_persisted("0".repeat(64))
            .unwrap(),
        IdempotencyId::parse("correction-execute").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    let error = h
        .service
        .approve_and_execute_record_risk_occurrence(ApproveAndExecuteRecordRiskOccurrence {
            approval: invalid,
            context: ctx("correction-execute"),
        })
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::SecurityPreviewExpiredOrChanged);

    let corrected = execute_occurrence(&mut h, &prepared, "correction-execute").unwrap();
    assert_eq!(
        corrected.issue.id(),
        &IssueId::parse("issue-correction").unwrap()
    );
}

#[test]
fn stale_evidence_is_attention_only_and_does_not_block_approved_occurrence() {
    let mut h = harness();
    let (_, prepared) = prepared_occurrence(&mut h, "attention-prepare", "issue-attention");
    h.evidence.set(0);
    let outcome = execute_occurrence(&mut h, &prepared, "attention-execute").unwrap();
    assert_eq!(
        outcome.issue.id(),
        &IssueId::parse("issue-attention").unwrap()
    );
}

#[test]
fn prepare_policy_denial_audit_is_truthful_and_effect_free() {
    let mut h = harness();
    let risk = h
        .service
        .create_risk(create("prepare-deny-create"))
        .unwrap()
        .record;
    h.allowed.set(false);
    let error = h
        .service
        .prepare_record_risk_occurrence(PrepareRecordRiskOccurrence {
            risk_id: risk.id().clone(),
            expected_version: risk.version(),
            issue_id: IssueId::parse("issue-prepare-deny").unwrap(),
            context: ctx("prepare-deny"),
        })
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::SecurityPolicyDenied);
    let audit = h.service.audit_events().last().unwrap();
    assert_eq!(audit.policy_outcome(), AuditPolicyOutcome::Denied);
    assert_eq!(audit.approval_outcome(), AuditApprovalOutcome::NotRequired);
    assert_eq!(
        audit.execution_outcome(),
        AuditExecutionOutcome::NotAttempted
    );
    assert_eq!(audit.effect_scope(), AuditEffectScope::None);
    assert!(audit.actual_effects().is_empty());
}

#[test]
fn execute_denials_have_truthful_metadata_only_audits() {
    let cases = [
        "policy",
        "unauthorized",
        "digest",
        "expiry",
        "stale",
        "classification",
        "commit",
    ];
    for case in cases {
        let mut h = harness();
        let (risk, p) =
            prepared_occurrence(&mut h, &format!("{case}-prepare"), &format!("issue-{case}"));
        let mut a = approval(&p, &format!("{case}-execute"));
        match case {
            "policy" => h.allowed.set(false),
            "unauthorized" => h.authorized.set(false),
            "digest" => {
                let p2 = h
                    .service
                    .prepare_close_risk(PrepareCloseRisk {
                        risk_id: risk.id().clone(),
                        expected_version: risk.version(),
                        rationale: txt("close"),
                        context: ctx("digest-other"),
                    })
                    .unwrap();
                a = WorkManagementApproval::new(
                    p.id().clone(),
                    AuditActor::HeadOfProducts,
                    p2.payload_digest().clone(),
                    IdempotencyId::parse("digest-execute").unwrap(),
                    Some(ApprovalConfirmation::Confirmed),
                )
                .unwrap();
            }
            "expiry" => h.time.set(p.preview().expires_at().unix_millis()),
            "stale" => {
                h.service
                    .update_risk_response(UpdateRiskResponse {
                        risk_id: risk.id().clone(),
                        expected_version: risk.version(),
                        response: RiskResponseType::Mitigate,
                        owner: None,
                        rationale: None,
                        residual_exposure: None,
                        next_review_at: None,
                        context: ctx("make-stale"),
                    })
                    .unwrap();
            }
            "classification" => {
                h.classification.set(DataClassification::Restricted);
            }
            "commit" => h.service.inject_next_commit_failure(),
            _ => unreachable!(),
        }
        let before_version = h.service.risk(risk.id()).unwrap().version();
        let result = h.service.approve_and_execute_record_risk_occurrence(
            ApproveAndExecuteRecordRiskOccurrence {
                approval: a,
                context: ctx(&format!("{case}-execute")),
            },
        );
        assert!(result.is_err(), "{case}");
        assert_eq!(
            h.service.risk(risk.id()).unwrap().version(),
            before_version,
            "{case}"
        );
        assert!(h
            .service
            .issue(&IssueId::parse(format!("issue-{case}")).unwrap())
            .is_none());
        let audit = h.service.audit_events().last().unwrap();
        assert_eq!(audit.effect_scope(), AuditEffectScope::None, "{case}");
        assert!(audit.actual_effects().is_empty());
        match case {
            "policy" => {
                assert_eq!(audit.policy_outcome(), AuditPolicyOutcome::Denied);
                assert_eq!(audit.approval_outcome(), AuditApprovalOutcome::NotRequired);
                assert_eq!(
                    audit.execution_outcome(),
                    AuditExecutionOutcome::NotAttempted
                );
            }
            "commit" => {
                assert_eq!(audit.policy_outcome(), AuditPolicyOutcome::Allowed);
                assert_eq!(audit.approval_outcome(), AuditApprovalOutcome::Approved);
                assert_eq!(audit.execution_outcome(), AuditExecutionOutcome::Failed);
            }
            _ => {
                assert_eq!(audit.policy_outcome(), AuditPolicyOutcome::Allowed);
                assert_eq!(audit.approval_outcome(), AuditApprovalOutcome::Rejected);
                assert_eq!(
                    audit.execution_outcome(),
                    AuditExecutionOutcome::NotAttempted
                );
            }
        }
    }
}

#[test]
fn unclassified_and_audit_id_failure_fail_closed_without_business_effect_and_retry_works() {
    let mut h = harness();
    let (_risk, p) = prepared_occurrence(&mut h, "fail-prepare", "issue-fail");
    h.classification.set(DataClassification::Unclassified);
    let error = execute_occurrence(&mut h, &p, "unclassified-execute").unwrap_err();
    assert_eq!(error.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
    h.classification.set(DataClassification::Internal);
    h.fail_audit.set(true);
    h.service.inject_next_commit_failure();
    let error = execute_occurrence(&mut h, &p, "audit-fail-execute").unwrap_err();
    assert_eq!(error.code(), ErrorCode::PlatformInternal);
    assert!(h
        .service
        .issue(&IssueId::parse("issue-fail").unwrap())
        .is_none());
    let commit_error = execute_occurrence(&mut h, &p, "audit-fail-execute").unwrap_err();
    assert_eq!(commit_error.code(), ErrorCode::PlatformInternal);
    assert!(h
        .service
        .issue(&IssueId::parse("issue-fail").unwrap())
        .is_none());
    let out = execute_occurrence(&mut h, &p, "audit-fail-execute").unwrap();
    assert_eq!(out.issue.id(), &IssueId::parse("issue-fail").unwrap());
}

#[test]
fn h1_stale_error_preserves_caller_correlation_current_version_and_remediation_key() {
    let mut s = service();
    let risk = s.create_risk(create("h1-create")).unwrap().record;
    let error = s
        .update_risk_response(UpdateRiskResponse {
            risk_id: risk.id().clone(),
            expected_version: AggregateVersion::initial().next().unwrap(),
            response: RiskResponseType::Mitigate,
            owner: None,
            rationale: None,
            residual_exposure: None,
            next_review_at: None,
            context: ctx("h1-stale"),
        })
        .unwrap_err();
    assert_eq!(error.correlation_id(), &ctx("h1-stale").correlation_id);
    assert_eq!(error.message_key().as_str(), "risk.stale_or_illegal");
    assert!(
        matches!(error.extensions(), [SafeErrorExtension::CurrentVersion(v)] if *v == risk.version())
    );
    assert!(error
        .params()
        .iter()
        .any(|param| param.key() == "current_state"
            && matches!(param.value(), SafeParamValue::Identifier(value) if value == "open")));
    assert!(error.params().iter().any(|param| param.key() == "allowed_next_intents"
        && matches!(param.value(), SafeParamValue::FieldKey(value) if value == "risk.update_response_or_prepare_transition")));
    assert!(error.params().iter().any(|param| param.key() == "remediation"
        && matches!(param.value(), SafeParamValue::FieldKey(value) if value == "risk.refresh_and_reprepare")));
}

#[test]
fn h1_terminal_conflict_reports_state_no_transition_and_reprepare_guidance() {
    let mut s = service();
    let risk = s.create_risk(create("terminal-create")).unwrap().record;
    let prepared = s
        .prepare_close_risk(PrepareCloseRisk {
            risk_id: risk.id().clone(),
            expected_version: risk.version(),
            rationale: txt("done"),
            context: ctx("terminal-prepare"),
        })
        .unwrap();
    let closed = s
        .approve_and_execute_close_risk(ApproveAndExecuteCloseRisk {
            approval: approval(&prepared, "terminal-execute"),
            context: ctx("terminal-execute"),
        })
        .unwrap()
        .record;
    let error = s
        .update_risk_response(UpdateRiskResponse {
            risk_id: closed.id().clone(),
            expected_version: closed.version(),
            response: RiskResponseType::Mitigate,
            owner: None,
            rationale: None,
            residual_exposure: None,
            next_review_at: None,
            context: ctx("terminal-update"),
        })
        .unwrap_err();
    assert!(error
        .params()
        .iter()
        .any(|param| param.key() == "current_state"
            && matches!(param.value(), SafeParamValue::Identifier(value) if value == "closed")));
    assert!(error.params().iter().any(|param| param.key() == "allowed_next_intents"
        && matches!(param.value(), SafeParamValue::FieldKey(value) if value == "risk.no_transition")));
    assert!(error.params().iter().any(|param| param.key() == "remediation"
        && matches!(param.value(), SafeParamValue::FieldKey(value) if value == "risk.refresh_and_reprepare")));
}

fn assert_prepare_lifecycle_error(
    error: &DomainError,
    state: &str,
    version: AggregateVersion,
    correlation_id: &CorrelationId,
    allowed_next: &str,
) {
    assert_eq!(error.code(), ErrorCode::DomainConflict);
    assert_eq!(error.message_key().as_str(), "risk.stale_or_illegal");
    assert_eq!(error.correlation_id(), correlation_id);
    assert!(
        matches!(error.extensions(), [SafeErrorExtension::CurrentVersion(current)] if *current == version)
    );
    assert!(error
        .params()
        .iter()
        .any(|param| param.key() == "current_state"
            && matches!(param.value(), SafeParamValue::Identifier(value) if value == state)));
    assert!(error
        .params()
        .iter()
        .any(|param| param.key() == "allowed_next_intents"
            && matches!(param.value(), SafeParamValue::FieldKey(value) if value == allowed_next)));
    assert!(error.params().iter().any(|param| param.key() == "remediation"
        && matches!(param.value(), SafeParamValue::FieldKey(value) if value == "risk.refresh_and_reprepare")));
}

#[test]
fn stale_occurrence_and_close_prepare_return_exact_lifecycle_metadata() {
    let mut s = service();
    let risk = s
        .create_risk(create("prepare-stale-create"))
        .unwrap()
        .record;
    let stale_version = risk.version().next().unwrap();
    let occurrence_context = ctx("prepare-stale-occurrence");
    let occurrence = s
        .prepare_record_risk_occurrence(PrepareRecordRiskOccurrence {
            risk_id: risk.id().clone(),
            expected_version: stale_version,
            issue_id: IssueId::parse("issue-stale-prepare").unwrap(),
            context: occurrence_context.clone(),
        })
        .unwrap_err();
    assert_prepare_lifecycle_error(
        &occurrence,
        "open",
        risk.version(),
        &occurrence_context.correlation_id,
        "risk.update_response_or_prepare_transition",
    );
    let close_context = ctx("prepare-stale-close");
    let close = s
        .prepare_close_risk(PrepareCloseRisk {
            risk_id: risk.id().clone(),
            expected_version: stale_version,
            rationale: txt("close"),
            context: close_context.clone(),
        })
        .unwrap_err();
    assert_prepare_lifecycle_error(
        &close,
        "open",
        risk.version(),
        &close_context.correlation_id,
        "risk.update_response_or_prepare_transition",
    );
}

#[test]
fn terminal_occurrence_and_close_prepare_return_exact_lifecycle_metadata() {
    let mut s = service();
    let risk = s
        .create_risk(create("prepare-terminal-create"))
        .unwrap()
        .record;
    let prepared = s
        .prepare_close_risk(PrepareCloseRisk {
            risk_id: risk.id().clone(),
            expected_version: risk.version(),
            rationale: txt("done"),
            context: ctx("prepare-terminal-initial"),
        })
        .unwrap();
    let closed = s
        .approve_and_execute_close_risk(ApproveAndExecuteCloseRisk {
            approval: approval(&prepared, "prepare-terminal-execute"),
            context: ctx("prepare-terminal-execute"),
        })
        .unwrap()
        .record;
    let occurrence_context = ctx("terminal-occurrence-prepare");
    let occurrence = s
        .prepare_record_risk_occurrence(PrepareRecordRiskOccurrence {
            risk_id: closed.id().clone(),
            expected_version: closed.version(),
            issue_id: IssueId::parse("issue-terminal-prepare").unwrap(),
            context: occurrence_context.clone(),
        })
        .unwrap_err();
    assert_prepare_lifecycle_error(
        &occurrence,
        "closed",
        closed.version(),
        &occurrence_context.correlation_id,
        "risk.no_transition",
    );
    let close_context = ctx("terminal-close-prepare");
    let close = s
        .prepare_close_risk(PrepareCloseRisk {
            risk_id: closed.id().clone(),
            expected_version: closed.version(),
            rationale: txt("again"),
            context: close_context.clone(),
        })
        .unwrap_err();
    assert_prepare_lifecycle_error(
        &close,
        "closed",
        closed.version(),
        &close_context.correlation_id,
        "risk.no_transition",
    );
}
