use pmc_domain::audit::AuditActor;
use pmc_domain::classification::DataClassification;
use pmc_domain::identity::{
    ActionId, ActionRequestId, AggregateVersion, DecisionId, DecisionRequestId,
    EvidenceReferenceId, IdempotencyId, IssueId, PreparedIntentId, RiskId, StakeholderId,
};
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::*;
use pmc_domain::BoundedText;

fn action(value: &str) -> ActionId {
    ActionId::parse(value).unwrap_or_else(|e| panic!("id: {e}"))
}
fn prepared_id(value: &str) -> PreparedIntentId {
    PreparedIntentId::parse(value).unwrap_or_else(|e| panic!("id: {e}"))
}
fn accepted_subject(value: &str) -> BoundedText<240> {
    BoundedText::parse(value).unwrap_or_else(|e| panic!("subject: {e}"))
}
fn accepted_commitment(value: &str) -> BoundedText<2_000> {
    BoundedText::parse(value).unwrap_or_else(|e| panic!("commitment: {e}"))
}
fn accepted_owner(value: &str) -> StakeholderId {
    StakeholderId::parse(value).unwrap_or_else(|e| panic!("owner: {e}"))
}
fn decision_text(value: &str) -> BoundedText<4_000> {
    BoundedText::parse(value).unwrap_or_else(|e| panic!("decision text: {e}"))
}
fn resulting_request(value: &str) -> DecisionResultingActionRequest {
    DecisionResultingActionRequest {
        id: ActionRequestId::parse(value).unwrap_or_else(|e| panic!("id: {e}")),
        subject: accepted_subject("Synthetic resulting request"),
        details: accepted_commitment("Synthetic resulting request details"),
        intended_owner: accepted_owner("owner-resulting"),
        due_at: UtcTimestamp::from_unix_millis(12_000),
        classification: DataClassification::Public,
    }
}
fn evidence_binding(
    value: &str,
    classification: DataClassification,
) -> EvidenceClassificationBinding {
    EvidenceClassificationBinding::new(
        EvidenceReferenceId::parse(value).unwrap_or_else(|e| panic!("evidence id: {e}")),
        classification,
    )
}
fn digest(value: char) -> IntegrityDigest {
    IntegrityDigest::parse(value.to_string().repeat(64)).unwrap_or_else(|e| panic!("digest: {e}"))
}
fn evidence(
    value: &str,
    class: DataClassification,
    verification: EvidenceVerification,
) -> EvidenceReferenceMetadata {
    EvidenceReferenceMetadata::new(
        EvidenceReferenceId::parse(value).unwrap_or_else(|e| panic!("id: {e}")),
        AggregateVersion::initial(),
        class,
        EvidenceRole::ActionCompletion,
        verification,
    )
}
fn evidence_for(
    value: &str,
    class: DataClassification,
    role: EvidenceRole,
    verification: EvidenceVerification,
) -> EvidenceReferenceMetadata {
    EvidenceReferenceMetadata::new(
        EvidenceReferenceId::parse(value).unwrap_or_else(|e| panic!("id: {e}")),
        AggregateVersion::initial(),
        class,
        role,
        verification,
    )
}
fn judgment(rationale: &str, class: DataClassification) -> HumanJudgment {
    HumanJudgment::new(
        HumanJudgmentDisposition::ProceedWithDocumentedRationale,
        rationale,
        class,
    )
    .unwrap_or_else(|e| panic!("judgment: {e}"))
}
fn decision_support() -> SupportWitness {
    EvidenceOrJudgment::new(
        vec![],
        vec![judgment(
            "Synthetic decision rationale.",
            DataClassification::Public,
        )],
    )
    .unwrap_or_else(|e| panic!("{e}"))
    .evaluate_evidence_or_judgment()
    .unwrap_or_else(|e| panic!("{e}"))
}
fn issue_resolution_support() -> SupportWitness {
    EvidenceOrJudgment::new(
        vec![evidence_for(
            "issue-resolution-evidence",
            DataClassification::Internal,
            EvidenceRole::IssueResolution,
            EvidenceVerification::Verified {
                verified_at: UtcTimestamp::from_unix_millis(900),
                integrity_digest: digest('c'),
            },
        )],
        vec![],
    )
    .unwrap()
    .evaluate_evidence_required()
    .unwrap()
}

#[test]
fn lifecycle_values_round_trip_and_unknown_fails_closed() {
    macro_rules! roundtrip { ($ty:ty, [$($v:expr),+]) => { $(assert_eq!(<$ty>::from_persisted($v.as_persisted()), Ok($v));)+ }; }
    roundtrip!(
        ActionRequestState,
        [
            ActionRequestState::Draft,
            ActionRequestState::Open,
            ActionRequestState::Accepted,
            ActionRequestState::Declined,
            ActionRequestState::Withdrawn
        ]
    );
    roundtrip!(
        ActionState,
        [
            ActionState::Open,
            ActionState::InProgress,
            ActionState::Completed,
            ActionState::Cancelled
        ]
    );
    roundtrip!(
        DecisionRequestState,
        [
            DecisionRequestState::Draft,
            DecisionRequestState::Open,
            DecisionRequestState::Resolved,
            DecisionRequestState::Withdrawn
        ]
    );
    roundtrip!(
        DecisionState,
        [DecisionState::Effective, DecisionState::Superseded]
    );
    roundtrip!(
        RiskState,
        [RiskState::Open, RiskState::Occurred, RiskState::Closed]
    );
    roundtrip!(
        IssueState,
        [IssueState::Open, IssueState::Resolved, IssueState::Closed]
    );
    roundtrip!(
        IssueResolutionType,
        [
            IssueResolutionType::Resolved,
            IssueResolutionType::Workaround,
            IssueResolutionType::AcceptedImpact
        ]
    );
    roundtrip!(
        RiskResponseType,
        [
            RiskResponseType::Mitigate,
            RiskResponseType::Accept,
            RiskResponseType::Transfer,
            RiskResponseType::Avoid
        ]
    );
    assert!(ActionState::from_persisted("done").is_err());
}

#[test]
fn support_witness_truth_table_is_fail_closed() {
    let verified = evidence(
        "verified",
        DataClassification::Internal,
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(10),
            integrity_digest: digest('a'),
        },
    );
    let degraded = evidence(
        "degraded",
        DataClassification::Confidential,
        EvidenceVerification::DegradedLastVerified {
            last_verified_at: UtcTimestamp::from_unix_millis(9),
            integrity_digest: digest('b'),
        },
    );
    let unverified = evidence(
        "unverified",
        DataClassification::Public,
        EvidenceVerification::Unverified,
    );
    let mismatch = evidence(
        "mismatch",
        DataClassification::Restricted,
        EvidenceVerification::IntegrityMismatch,
    );
    let human = judgment(
        "Synthetic Head of Products rationale.",
        DataClassification::Confidential,
    );
    assert_eq!(
        EvidenceOrJudgment::new(vec![verified.clone()], vec![])
            .unwrap_or_else(|e| panic!("{e}"))
            .evaluate_evidence_required()
            .unwrap_or_else(|e| panic!("{e}"))
            .disposition(),
        SupportDisposition::EvidenceSatisfied
    );
    assert!(EvidenceOrJudgment::new(vec![], vec![]).is_err());
    for invalid in [unverified, mismatch] {
        let support =
            EvidenceOrJudgment::new(vec![invalid], vec![]).unwrap_or_else(|e| panic!("{e}"));
        assert!(support.evaluate_evidence_required().is_err());
        assert!(support.evaluate_evidence_or_judgment().is_err());
    }
    for selected in [
        vec![
            verified.clone(),
            evidence(
                "invalid-after",
                DataClassification::Public,
                EvidenceVerification::Unverified,
            ),
        ],
        vec![
            evidence(
                "invalid-before",
                DataClassification::Public,
                EvidenceVerification::IntegrityMismatch,
            ),
            verified,
        ],
    ] {
        let support = EvidenceOrJudgment::new(selected, vec![human.clone()])
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(support.evaluate_evidence_required().is_err());
        assert!(support.evaluate_evidence_or_judgment().is_err());
    }
    assert!(EvidenceOrJudgment::new(vec![degraded.clone()], vec![])
        .unwrap_or_else(|e| panic!("{e}"))
        .evaluate_evidence_required()
        .is_err());
    assert_eq!(
        EvidenceOrJudgment::new(vec![degraded], vec![human.clone()])
            .unwrap_or_else(|e| panic!("{e}"))
            .evaluate_evidence_required()
            .unwrap_or_else(|e| panic!("{e}"))
            .disposition(),
        SupportDisposition::VerificationPending
    );
    assert!(EvidenceOrJudgment::new(vec![], vec![human.clone()])
        .unwrap_or_else(|e| panic!("{e}"))
        .evaluate_evidence_required()
        .is_err());
    assert_eq!(
        EvidenceOrJudgment::new(vec![], vec![human])
            .unwrap_or_else(|e| panic!("{e}"))
            .evaluate_evidence_or_judgment()
            .unwrap_or_else(|e| panic!("{e}"))
            .disposition(),
        SupportDisposition::JudgmentSatisfied
    );
}

fn complete(
    witness: SupportWitness,
    target_class: DataClassification,
) -> WorkManagementPreparedIntent {
    WorkManagementPreparedIntent::prepare(
        prepared_id("prepared-action"),
        WorkManagementOperation::CompleteAction {
            action_id: action("action-alpha"),
            action_version: AggregateVersion::initial(),
        },
        target_class,
        Some(witness),
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap_or_else(|e| panic!("prepare: {e}"))
}

#[test]
fn closed_operation_topology_and_support_requirements_cannot_be_recombined() {
    assert_eq!(WorkManagementH2aIntentKind::ALL.len(), 23);
    assert!(WorkManagementH2aIntentKind::from_persisted("remove_relationship").is_err());
    let witness = EvidenceOrJudgment::new(
        vec![evidence(
            "verified",
            DataClassification::Internal,
            EvidenceVerification::Verified {
                verified_at: UtcTimestamp::from_unix_millis(10),
                integrity_digest: digest('a'),
            },
        )],
        vec![],
    )
    .unwrap_or_else(|e| panic!("{e}"))
    .evaluate_evidence_required()
    .unwrap_or_else(|e| panic!("{e}"));
    let prepared = complete(witness.clone(), DataClassification::Public);
    assert_eq!(prepared.classification(), DataClassification::Internal);
    assert_eq!(
        prepared.payload_digest(),
        &prepared.preview().payload_digest()
    );
    assert!(
        matches!(prepared.operation(), WorkManagementOperation::CompleteAction { action_id, action_version } if action_id.as_str() == "action-alpha" && *action_version == AggregateVersion::initial())
    );
    assert_eq!(
        WorkManagementPreparedIntent::prepare(
            prepared_id("missing"),
            WorkManagementOperation::CompleteAction {
                action_id: action("action-alpha"),
                action_version: AggregateVersion::initial()
            },
            DataClassification::Public,
            None,
            UtcTimestamp::from_unix_millis(1_000)
        ),
        Err(PreparedIntentError::InvalidSupport)
    );
    assert_eq!(
        WorkManagementPreparedIntent::prepare(
            prepared_id("extra"),
            WorkManagementOperation::CancelAction {
                action_id: action("action-alpha"),
                action_version: AggregateVersion::initial(),
                reason: WorkManagementRationale::parse("Synthetic cancellation reason.")
                    .unwrap_or_else(|e| panic!("reason: {e}")),
                evidence_classifications: vec![],
            },
            DataClassification::Public,
            Some(witness),
            UtcTimestamp::from_unix_millis(1_000)
        ),
        Err(PreparedIntentError::InvalidSupport)
    );
    let request = ActionRequestId::parse("request-alpha").unwrap_or_else(|e| panic!("id: {e}"));
    let created_action = action("action-created");
    let accept = WorkManagementPreparedIntent::prepare(
        prepared_id("prepared-accept"),
        WorkManagementOperation::AcceptActionRequest {
            request_id: request.clone(),
            request_version: AggregateVersion::initial(),
            action_id: created_action.clone(),
            action_classification: DataClassification::Public,
            action_subject: accepted_subject("Accepted synthetic action"),
            commitment_details: accepted_commitment("Synthetic commitment"),
            intended_owner: accepted_owner("owner-accept"),
            intended_due_at: UtcTimestamp::from_unix_millis(9_000),
        },
        DataClassification::Public,
        None,
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap_or_else(|e| panic!("prepare: {e}"));
    assert_eq!(
        accept.targets(),
        vec![WorkManagementTarget::ActionRequest(
            request.clone(),
            AggregateVersion::initial()
        )]
    );
    assert_eq!(
        accept.effects(),
        vec![
            WorkManagementEffect::AcceptActionRequest(request.clone()),
            WorkManagementEffect::CreateAction(created_action.clone()),
            WorkManagementEffect::LinkActionRequestToAction(request.clone(), created_action)
        ]
    );
    assert!(WorkManagementPreparedIntent::prepare(
        prepared_id("empty-downstream"),
        WorkManagementOperation::SupersedeDecision {
            decision_id: DecisionId::parse("decision-old").unwrap_or_else(|e| panic!("id: {e}")),
            decision_version: AggregateVersion::initial(),
            replacement_decision_id: DecisionId::parse("decision-new")
                .unwrap_or_else(|e| panic!("id: {e}")),
            replacement_decision_version: AggregateVersion::initial(),
            replacement_decision_classification: DataClassification::Public,
            replacement_statement: decision_text("Replacement statement"),
            replacement_rationale: decision_text("Replacement rationale"),
            replacement_impact: decision_text("Replacement impact"),
            replacement_owner: accepted_owner("owner-replacement"),
            replacement_decided_at: UtcTimestamp::from_unix_millis(10_000),
            resulting_action_requests: vec![],
            incomplete_downstream: vec![],
        },
        DataClassification::Public,
        Some(decision_support()),
        UtcTimestamp::from_unix_millis(1_000),
    )
    .is_ok());
}

#[test]
fn preparation_derives_contract_expiry_and_denies_unclassified_origins() {
    assert_eq!(SUPPORTED_WORK_MANAGEMENT_H2A_CONTRACT_VERSION, 1);
    assert_eq!(WORK_MANAGEMENT_H2A_TTL_MILLIS, 300_000);
    let request = ActionRequestId::parse("request-classification").unwrap();
    let operation = |action_classification| WorkManagementOperation::AcceptActionRequest {
        request_id: request.clone(),
        request_version: AggregateVersion::initial(),
        action_id: action("action-classification"),
        action_classification,
        action_subject: accepted_subject("Classification action"),
        commitment_details: accepted_commitment("Classification commitment"),
        intended_owner: accepted_owner("owner-classification"),
        intended_due_at: UtcTimestamp::from_unix_millis(9_000),
    };
    let prepared = WorkManagementPreparedIntent::prepare(
        prepared_id("derived-envelope"),
        operation(DataClassification::Restricted),
        DataClassification::Public,
        None,
        UtcTimestamp::from_unix_millis(42),
    )
    .unwrap();
    assert_eq!(prepared.preview().contract_version(), 1);
    assert_eq!(prepared.preview().expires_at().unix_millis(), 300_042);
    assert_eq!(prepared.classification(), DataClassification::Restricted);
    assert_eq!(
        prepared.payload_digest(),
        &prepared.preview().payload_digest()
    );
    assert_eq!(
        WorkManagementPreparedIntent::prepare(
            prepared_id("unclassified-target"),
            operation(DataClassification::Public),
            DataClassification::Unclassified,
            None,
            UtcTimestamp::from_unix_millis(0),
        ),
        Err(PreparedIntentError::UnclassifiedBinding)
    );
    assert_eq!(
        WorkManagementPreparedIntent::prepare(
            prepared_id("unclassified-created"),
            operation(DataClassification::Unclassified),
            DataClassification::Public,
            None,
            UtcTimestamp::from_unix_millis(0),
        ),
        Err(PreparedIntentError::UnclassifiedBinding)
    );
    assert_eq!(
        WorkManagementPreparedIntent::prepare(
            prepared_id("expiry-overflow"),
            operation(DataClassification::Public),
            DataClassification::Public,
            None,
            UtcTimestamp::from_unix_millis(i64::MAX - WORK_MANAGEMENT_H2A_TTL_MILLIS + 1),
        ),
        Err(PreparedIntentError::ExpiryOverflow)
    );
}

#[test]
fn equal_resolved_classification_retains_distinct_primary_and_created_sources() {
    let prepare = |id: &str, primary| {
        WorkManagementPreparedIntent::prepare(
            prepared_id(id),
            WorkManagementOperation::AcceptActionRequest {
                request_id: ActionRequestId::parse("request-source-regression").unwrap(),
                request_version: AggregateVersion::initial(),
                action_id: action("action-source-regression"),
                action_classification: DataClassification::Restricted,
                action_subject: accepted_subject("Source regression action"),
                commitment_details: accepted_commitment("Source regression commitment"),
                intended_owner: accepted_owner("owner-source"),
                intended_due_at: UtcTimestamp::from_unix_millis(9_000),
            },
            primary,
            None,
            UtcTimestamp::from_unix_millis(0),
        )
        .unwrap()
    };
    let public_primary = prepare(
        "classification-source-regression",
        DataClassification::Public,
    );
    let internal_primary = prepare(
        "classification-source-regression",
        DataClassification::Internal,
    );
    assert_eq!(
        public_primary.classification(),
        DataClassification::Restricted
    );
    assert_eq!(
        internal_primary.classification(),
        DataClassification::Restricted
    );
    assert_ne!(
        public_primary.payload_digest(),
        internal_primary.payload_digest()
    );
    let sources = public_primary.preview().classification_sources();
    assert_eq!(sources.len(), 2);
    assert_eq!(
        sources[0].role(),
        &WorkManagementClassificationSourceRole::PrimaryTarget
    );
    assert_eq!(sources[0].classification(), DataClassification::Public);
    assert_eq!(
        sources[1].role(),
        &WorkManagementClassificationSourceRole::CreatedAction
    );
    assert_eq!(sources[1].classification(), DataClassification::Restricted);
}

#[test]
fn accepted_action_payload_fields_are_each_digest_bound() {
    let prepare = |subject: &str, commitment: &str, owner: &str, due: i64| {
        WorkManagementPreparedIntent::prepare(
            prepared_id("accepted-payload-digest"),
            WorkManagementOperation::AcceptActionRequest {
                request_id: ActionRequestId::parse("request-payload").unwrap(),
                request_version: AggregateVersion::initial(),
                action_id: action("action-payload"),
                action_classification: DataClassification::Internal,
                action_subject: accepted_subject(subject),
                commitment_details: accepted_commitment(commitment),
                intended_owner: accepted_owner(owner),
                intended_due_at: UtcTimestamp::from_unix_millis(due),
            },
            DataClassification::Public,
            None,
            UtcTimestamp::from_unix_millis(0),
        )
        .unwrap()
    };
    let base = prepare("Subject A", "Commitment A", "owner-a", 10_000);
    for changed in [
        prepare("Subject B", "Commitment A", "owner-a", 10_000),
        prepare("Subject A", "Commitment B", "owner-a", 10_000),
        prepare("Subject A", "Commitment A", "owner-b", 10_000),
        prepare("Subject A", "Commitment A", "owner-a", 20_000),
    ] {
        assert_ne!(base.payload_digest(), changed.payload_digest());
    }
}

#[test]
fn degraded_evidence_has_precedence_and_requires_judgment_in_both_orders() {
    let verified = evidence(
        "verified-mixed",
        DataClassification::Public,
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(10),
            integrity_digest: digest('a'),
        },
    );
    let degraded = evidence(
        "degraded-mixed",
        DataClassification::Internal,
        EvidenceVerification::DegradedLastVerified {
            last_verified_at: UtcTimestamp::from_unix_millis(9),
            integrity_digest: digest('b'),
        },
    );
    for selected in [
        vec![verified.clone(), degraded.clone()],
        vec![degraded.clone(), verified.clone()],
    ] {
        assert!(EvidenceOrJudgment::new(selected.clone(), vec![])
            .unwrap()
            .evaluate_evidence_required()
            .is_err());
        let judgment = judgment(
            "Explicit degraded-mode judgment.",
            DataClassification::Internal,
        );
        assert_eq!(
            EvidenceOrJudgment::new(selected.clone(), vec![judgment.clone()])
                .unwrap()
                .evaluate_evidence_required()
                .unwrap()
                .disposition(),
            SupportDisposition::VerificationPending
        );
        assert_eq!(
            EvidenceOrJudgment::new(selected, vec![judgment])
                .unwrap()
                .evaluate_evidence_or_judgment()
                .unwrap()
                .disposition(),
            SupportDisposition::VerificationPending
        );
    }
}

#[test]
fn observed_unpinned_evidence_never_satisfies_a_gate_alone_and_pends_with_a_judgment() {
    // An unpinned observation is a visible limitation, not a verdict.
    // Alone it satisfies nothing; with a recorded human judgment it takes
    // the Degraded path to VerificationPending, in either order beside a
    // Verified reference, on both gates.
    let unpinned = evidence(
        "unpinned",
        DataClassification::Internal,
        EvidenceVerification::ObservedUnpinned {
            observed_at: UtcTimestamp::from_unix_millis(11),
            integrity_digest: digest('c'),
        },
    );
    let verified = evidence(
        "verified-beside",
        DataClassification::Public,
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(10),
            integrity_digest: digest('a'),
        },
    );
    for selected in [
        vec![unpinned.clone()],
        vec![verified.clone(), unpinned.clone()],
        vec![unpinned.clone(), verified],
    ] {
        let alone = EvidenceOrJudgment::new(selected.clone(), vec![]).unwrap();
        assert!(alone.evaluate_evidence_required().is_err());
        assert!(alone.evaluate_evidence_or_judgment().is_err());

        let human = judgment(
            "Reviewed by hand; pin pending.",
            DataClassification::Internal,
        );
        let judged = EvidenceOrJudgment::new(selected, vec![human]).unwrap();
        assert_eq!(
            judged.evaluate_evidence_required().unwrap().disposition(),
            SupportDisposition::VerificationPending
        );
        assert_eq!(
            judged
                .evaluate_evidence_or_judgment()
                .unwrap()
                .disposition(),
            SupportDisposition::VerificationPending
        );
    }
}

#[test]
fn observed_unpinned_round_trips_through_its_persisted_parts_and_requires_both() {
    let state = EvidenceVerification::ObservedUnpinned {
        observed_at: UtcTimestamp::from_unix_millis(11),
        integrity_digest: digest('c'),
    };
    assert_eq!(state.kind_as_persisted(), "observed_unpinned");
    assert_eq!(
        EvidenceVerification::from_persisted_parts(
            "observed_unpinned",
            Some(UtcTimestamp::from_unix_millis(11)),
            Some(digest('c')),
        )
        .unwrap(),
        state
    );
    assert!(EvidenceVerification::from_persisted_parts(
        "observed_unpinned",
        None,
        Some(digest('c'))
    )
    .is_err());
    assert!(EvidenceVerification::from_persisted_parts(
        "observed_unpinned",
        Some(UtcTimestamp::from_unix_millis(11)),
        None
    )
    .is_err());
}

#[test]
fn created_record_sources_and_support_are_explicitly_classified() {
    let decision_support = EvidenceOrJudgment::new(
        vec![],
        vec![judgment(
            "Synthetic classified decision judgment.",
            DataClassification::Internal,
        )],
    )
    .unwrap()
    .evaluate_evidence_or_judgment()
    .unwrap();
    let resolve = WorkManagementOperation::ResolveDecisionRequest {
        request_id: DecisionRequestId::parse("decision-request-classification").unwrap(),
        request_version: AggregateVersion::initial(),
        decision_id: DecisionId::parse("decision-created-classification").unwrap(),
        decision_classification: DataClassification::Unclassified,
        statement: decision_text("Decision statement"),
        rationale: decision_text("Decision rationale"),
        impact: decision_text("Decision impact"),
        decision_owner: accepted_owner("owner-decision"),
        decided_at: UtcTimestamp::from_unix_millis(10_000),
        resulting_action_requests: vec![],
    };
    assert_eq!(
        WorkManagementPreparedIntent::prepare(
            prepared_id("unclassified-decision"),
            resolve,
            DataClassification::Public,
            Some(decision_support),
            UtcTimestamp::from_unix_millis(0),
        ),
        Err(PreparedIntentError::UnclassifiedBinding)
    );
    let unclassified_support = EvidenceOrJudgment::new(
        vec![],
        vec![judgment(
            "Synthetic unclassified support must be denied.",
            DataClassification::Unclassified,
        )],
    )
    .unwrap()
    .evaluate_evidence_or_judgment()
    .unwrap();
    assert_eq!(
        WorkManagementPreparedIntent::prepare(
            prepared_id("unclassified-support"),
            WorkManagementOperation::ResolveDecisionRequest {
                request_id: DecisionRequestId::parse("decision-request-support").unwrap(),
                request_version: AggregateVersion::initial(),
                decision_id: DecisionId::parse("decision-support").unwrap(),
                decision_classification: DataClassification::Public,
                statement: decision_text("Decision statement"),
                rationale: decision_text("Decision rationale"),
                impact: decision_text("Decision impact"),
                decision_owner: accepted_owner("owner-decision"),
                decided_at: UtcTimestamp::from_unix_millis(10_000),
                resulting_action_requests: vec![],
            },
            DataClassification::Public,
            Some(unclassified_support),
            UtcTimestamp::from_unix_millis(0),
        ),
        Err(PreparedIntentError::UnclassifiedBinding)
    );
    let record = WorkManagementOperation::RecordRiskOccurrence {
        risk_id: RiskId::parse("risk-classification").unwrap(),
        risk_version: AggregateVersion::initial(),
        issue_id: IssueId::parse("issue-created-classification").unwrap(),
        issue_classification: DataClassification::Unclassified,
    };
    assert_eq!(
        WorkManagementPreparedIntent::prepare(
            prepared_id("unclassified-issue"),
            record,
            DataClassification::Public,
            None,
            UtcTimestamp::from_unix_millis(0),
        ),
        Err(PreparedIntentError::UnclassifiedBinding)
    );
}

#[test]
fn supersede_allows_no_downstream_but_rejects_self_replacement() {
    let decision = DecisionId::parse("decision-self").unwrap();
    let result = WorkManagementPreparedIntent::prepare(
        prepared_id("self-replacement"),
        WorkManagementOperation::SupersedeDecision {
            decision_id: decision.clone(),
            decision_version: AggregateVersion::initial(),
            replacement_decision_id: decision,
            replacement_decision_version: AggregateVersion::initial(),
            replacement_decision_classification: DataClassification::Public,
            replacement_statement: decision_text("Replacement statement"),
            replacement_rationale: decision_text("Replacement rationale"),
            replacement_impact: decision_text("Replacement impact"),
            replacement_owner: accepted_owner("owner-replacement"),
            replacement_decided_at: UtcTimestamp::from_unix_millis(10_000),
            resulting_action_requests: vec![],
            incomplete_downstream: vec![],
        },
        DataClassification::Public,
        Some(decision_support()),
        UtcTimestamp::from_unix_millis(0),
    );
    assert_eq!(result, Err(PreparedIntentError::InvalidTopology));
}

#[test]
fn cancel_evidence_classification_bindings_are_canonical_exact_and_fail_closed() {
    let prepare = |id: &str, bindings| {
        WorkManagementPreparedIntent::prepare(
            prepared_id(id),
            WorkManagementOperation::CancelAction {
                action_id: action("action-evidence-binding"),
                action_version: AggregateVersion::initial(),
                reason: WorkManagementRationale::parse("Synthetic cancellation.").unwrap(),
                evidence_classifications: bindings,
            },
            DataClassification::Restricted,
            None,
            UtcTimestamp::from_unix_millis(0),
        )
    };
    let public = prepare(
        "cancel-binding",
        vec![evidence_binding("evidence-b", DataClassification::Public)],
    )
    .unwrap();
    let internal = prepare(
        "cancel-binding",
        vec![evidence_binding("evidence-b", DataClassification::Internal)],
    )
    .unwrap();
    assert_eq!(public.classification(), DataClassification::Restricted);
    assert_eq!(internal.classification(), DataClassification::Restricted);
    assert_ne!(public.payload_digest(), internal.payload_digest());
    assert!(public
        .preview()
        .classification_sources()
        .iter()
        .any(|source| {
            source.role()
                == &WorkManagementClassificationSourceRole::Evidence(
                    EvidenceReferenceId::parse("evidence-b").unwrap(),
                )
                && source.classification() == DataClassification::Public
        }));
    assert_eq!(
        prepare(
            "cancel-unclassified",
            vec![evidence_binding(
                "evidence-u",
                DataClassification::Unclassified
            )]
        ),
        Err(PreparedIntentError::UnclassifiedBinding)
    );
    assert_eq!(
        prepare(
            "cancel-duplicate",
            vec![
                evidence_binding("evidence-d", DataClassification::Public),
                evidence_binding("evidence-d", DataClassification::Internal),
            ]
        ),
        Err(PreparedIntentError::InvalidSupport)
    );
    let ordered_a = prepare(
        "cancel-order",
        vec![
            evidence_binding("evidence-z", DataClassification::Public),
            evidence_binding("evidence-a", DataClassification::Internal),
        ],
    )
    .unwrap();
    let ordered_b = prepare(
        "cancel-order",
        vec![
            evidence_binding("evidence-a", DataClassification::Internal),
            evidence_binding("evidence-z", DataClassification::Public),
        ],
    )
    .unwrap();
    assert_eq!(ordered_a.payload_digest(), ordered_b.payload_digest());
}

#[test]
fn digest_binds_every_mutable_envelope_and_closed_operation_field() {
    #[allow(clippy::too_many_arguments)]
    fn cancel(
        pid: &str,
        _version: u16,
        aid: &str,
        aggregate_version: u64,
        class: DataClassification,
        expiry: i64,
        preview: &str,
        reopen: bool,
    ) -> WorkManagementPreparedIntent {
        let versioned_id = (
            action(aid),
            AggregateVersion::new(aggregate_version).unwrap_or_else(|e| panic!("version: {e}")),
        );
        let operation = if reopen {
            WorkManagementOperation::ReopenAction {
                action_id: versioned_id.0,
                action_version: versioned_id.1,
                mode: ActionReopenMode::ReopenCompleted,
                reason: WorkManagementRationale::parse(preview)
                    .unwrap_or_else(|e| panic!("reason: {e}")),
                evidence_classifications: vec![],
            }
        } else {
            WorkManagementOperation::CancelAction {
                action_id: versioned_id.0,
                action_version: versioned_id.1,
                reason: WorkManagementRationale::parse(preview)
                    .unwrap_or_else(|e| panic!("reason: {e}")),
                evidence_classifications: vec![],
            }
        };
        WorkManagementPreparedIntent::prepare(
            prepared_id(pid),
            operation,
            class,
            None,
            UtcTimestamp::from_unix_millis(expiry),
        )
        .unwrap_or_else(|e| panic!("prepare: {e}"))
    }
    let base = cancel(
        "prepared-cancel",
        1,
        "action-alpha",
        1,
        DataClassification::Internal,
        1_000,
        "Cancel exact Action.",
        false,
    );
    let changed = [
        cancel(
            "prepared-other",
            1,
            "action-alpha",
            1,
            DataClassification::Internal,
            1_000,
            "Cancel exact Action.",
            false,
        ),
        cancel(
            "prepared-cancel",
            1,
            "action-beta",
            1,
            DataClassification::Internal,
            1_000,
            "Cancel exact Action.",
            false,
        ),
        cancel(
            "prepared-cancel",
            1,
            "action-alpha",
            2,
            DataClassification::Internal,
            1_000,
            "Cancel exact Action.",
            false,
        ),
        cancel(
            "prepared-cancel",
            1,
            "action-alpha",
            1,
            DataClassification::Restricted,
            1_000,
            "Cancel exact Action.",
            false,
        ),
        cancel(
            "prepared-cancel",
            1,
            "action-alpha",
            1,
            DataClassification::Internal,
            2_000,
            "Cancel exact Action.",
            false,
        ),
        cancel(
            "prepared-cancel",
            1,
            "action-alpha",
            1,
            DataClassification::Internal,
            1_000,
            "Changed preview.",
            false,
        ),
        cancel(
            "prepared-cancel",
            1,
            "action-alpha",
            1,
            DataClassification::Internal,
            1_000,
            "Cancel exact Action.",
            true,
        ),
    ];
    assert!(changed
        .iter()
        .all(|item| item.payload_digest() != base.payload_digest()));
}

#[test]
fn all_eleven_operations_have_exact_dg0_topology() {
    let v = AggregateVersion::initial();
    let ar = ActionRequestId::parse("request-a").unwrap_or_else(|e| panic!("id: {e}"));
    let a = action("action-a");
    let dr = DecisionRequestId::parse("decision-request-a").unwrap_or_else(|e| panic!("id: {e}"));
    let d = DecisionId::parse("decision-a").unwrap_or_else(|e| panic!("id: {e}"));
    let replacement = DecisionId::parse("decision-b").unwrap_or_else(|e| panic!("id: {e}"));
    let risk = RiskId::parse("risk-a").unwrap_or_else(|e| panic!("id: {e}"));
    let issue = IssueId::parse("issue-a").unwrap_or_else(|e| panic!("id: {e}"));
    let downstream_request =
        ActionRequestId::parse("request-downstream").unwrap_or_else(|e| panic!("id: {e}"));
    let downstream_action = action("action-downstream");
    let reason = || {
        WorkManagementRationale::parse("Synthetic bounded rationale.")
            .unwrap_or_else(|e| panic!("rationale: {e}"))
    };
    let verified = |role| {
        EvidenceOrJudgment::new(
            vec![evidence_for(
                "topology-evidence",
                DataClassification::Public,
                role,
                EvidenceVerification::Verified {
                    verified_at: UtcTimestamp::from_unix_millis(10),
                    integrity_digest: digest('c'),
                },
            )],
            vec![],
        )
        .unwrap_or_else(|e| panic!("{e}"))
        .evaluate_evidence_required()
        .unwrap_or_else(|e| panic!("{e}"))
    };
    let decision_support = EvidenceOrJudgment::new(
        vec![],
        vec![judgment(
            "Synthetic decision rationale.",
            DataClassification::Public,
        )],
    )
    .unwrap_or_else(|e| panic!("{e}"))
    .evaluate_evidence_or_judgment()
    .unwrap_or_else(|e| panic!("{e}"));

    let cases = vec![
        (
            WorkManagementOperation::AcceptActionRequest {
                request_id: ar.clone(),
                request_version: v,
                action_id: a.clone(),
                action_classification: DataClassification::Public,
                action_subject: accepted_subject("Topology action"),
                commitment_details: accepted_commitment("Topology commitment"),
                intended_owner: accepted_owner("owner-topology"),
                intended_due_at: UtcTimestamp::from_unix_millis(9_000),
            },
            None,
            vec![WorkManagementTarget::ActionRequest(ar.clone(), v)],
            vec![
                WorkManagementEffect::AcceptActionRequest(ar.clone()),
                WorkManagementEffect::CreateAction(a.clone()),
                WorkManagementEffect::LinkActionRequestToAction(ar.clone(), a.clone()),
            ],
        ),
        (
            WorkManagementOperation::ResolveDecisionRequest {
                request_id: dr.clone(),
                request_version: v,
                decision_id: d.clone(),
                decision_classification: DataClassification::Public,
                statement: decision_text("Decision statement"),
                rationale: decision_text("Decision rationale"),
                impact: decision_text("Decision impact"),
                decision_owner: accepted_owner("owner-decision"),
                decided_at: UtcTimestamp::from_unix_millis(10_000),
                resulting_action_requests: vec![resulting_request("request-from-resolution")],
            },
            Some(decision_support.clone()),
            vec![
                WorkManagementTarget::DecisionRequest(dr.clone(), v),
                WorkManagementTarget::Decision(d.clone(), AggregateVersion::initial()),
                WorkManagementTarget::ActionRequest(
                    ActionRequestId::parse("request-from-resolution").unwrap(),
                    AggregateVersion::initial(),
                ),
            ],
            vec![
                WorkManagementEffect::ResolveDecisionRequest(dr.clone()),
                WorkManagementEffect::CreateDecision(d.clone()),
                WorkManagementEffect::LinkDecisionRequestToDecision(dr.clone(), d.clone()),
                WorkManagementEffect::CreateResultingActionRequest(
                    ActionRequestId::parse("request-from-resolution").unwrap(),
                ),
                WorkManagementEffect::LinkDecisionToActionRequest(
                    d.clone(),
                    ActionRequestId::parse("request-from-resolution").unwrap(),
                ),
            ],
        ),
        (
            WorkManagementOperation::CompleteAction {
                action_id: a.clone(),
                action_version: v,
            },
            Some(verified(EvidenceRole::ActionCompletion)),
            vec![WorkManagementTarget::Action(a.clone(), v)],
            vec![WorkManagementEffect::CompleteAction(a.clone())],
        ),
        (
            WorkManagementOperation::CancelAction {
                action_id: a.clone(),
                action_version: v,
                reason: reason(),
                evidence_classifications: vec![],
            },
            None,
            vec![WorkManagementTarget::Action(a.clone(), v)],
            vec![WorkManagementEffect::CancelAction(a.clone())],
        ),
        (
            WorkManagementOperation::ReopenAction {
                action_id: a.clone(),
                action_version: v,
                mode: ActionReopenMode::RestartCancelled,
                reason: reason(),
                evidence_classifications: vec![],
            },
            None,
            vec![WorkManagementTarget::Action(a.clone(), v)],
            vec![WorkManagementEffect::ReopenAction(a.clone())],
        ),
        (
            WorkManagementOperation::SupersedeDecision {
                decision_id: d.clone(),
                decision_version: v,
                replacement_decision_id: replacement.clone(),
                replacement_decision_version: v,
                replacement_decision_classification: DataClassification::Public,
                replacement_statement: decision_text("Replacement statement"),
                replacement_rationale: decision_text("Replacement rationale"),
                replacement_impact: decision_text("Replacement impact"),
                replacement_owner: accepted_owner("owner-replacement"),
                replacement_decided_at: UtcTimestamp::from_unix_millis(10_000),
                resulting_action_requests: vec![resulting_request("request-from-replacement")],
                incomplete_downstream: vec![
                    IncompleteDownstreamWork::ActionRequest(
                        downstream_request.clone(),
                        v,
                        DataClassification::Public,
                    ),
                    IncompleteDownstreamWork::Action(
                        downstream_action.clone(),
                        v,
                        DataClassification::Public,
                    ),
                ],
            },
            Some(decision_support),
            vec![
                WorkManagementTarget::Decision(d.clone(), v),
                WorkManagementTarget::Decision(replacement.clone(), v),
                WorkManagementTarget::ActionRequest(
                    ActionRequestId::parse("request-from-replacement").unwrap(),
                    AggregateVersion::initial(),
                ),
                WorkManagementTarget::Action(downstream_action.clone(), v),
                WorkManagementTarget::ActionRequest(downstream_request.clone(), v),
            ],
            vec![
                WorkManagementEffect::SupersedeDecision(d.clone()),
                WorkManagementEffect::CreateDecision(replacement.clone()),
                WorkManagementEffect::LinkReplacementDecision(d.clone(), replacement.clone()),
                WorkManagementEffect::CreateResultingActionRequest(
                    ActionRequestId::parse("request-from-replacement").unwrap(),
                ),
                WorkManagementEffect::LinkDecisionToActionRequest(
                    replacement.clone(),
                    ActionRequestId::parse("request-from-replacement").unwrap(),
                ),
                WorkManagementEffect::FlagSupersededPremiseAction(downstream_action),
                WorkManagementEffect::FlagSupersededPremiseActionRequest(downstream_request),
            ],
        ),
        (
            WorkManagementOperation::RecordRiskOccurrence {
                risk_id: risk.clone(),
                risk_version: v,
                issue_id: issue.clone(),
                issue_classification: DataClassification::Public,
            },
            None,
            vec![WorkManagementTarget::Risk(risk.clone(), v)],
            vec![
                WorkManagementEffect::RecordRiskOccurrence(risk.clone()),
                WorkManagementEffect::CreateIssue(issue.clone()),
                WorkManagementEffect::LinkRiskToIssue(risk.clone(), issue.clone()),
            ],
        ),
        (
            WorkManagementOperation::CloseRisk {
                risk_id: risk.clone(),
                risk_version: v,
                rationale: reason(),
            },
            None,
            vec![WorkManagementTarget::Risk(risk.clone(), v)],
            vec![WorkManagementEffect::CloseRisk(risk)],
        ),
        (
            WorkManagementOperation::ResolveIssue {
                issue_id: issue.clone(),
                issue_version: v,
                resolution_type: IssueResolutionType::Resolved,
                rationale: reason(),
            },
            Some(verified(EvidenceRole::IssueResolution)),
            vec![WorkManagementTarget::Issue(issue.clone(), v)],
            vec![WorkManagementEffect::ResolveIssue(issue.clone())],
        ),
        (
            WorkManagementOperation::CloseIssue {
                issue_id: issue.clone(),
                issue_version: v,
            },
            Some(verified(EvidenceRole::IssueClosureVerification)),
            vec![WorkManagementTarget::Issue(issue.clone(), v)],
            vec![WorkManagementEffect::CloseIssue(issue.clone())],
        ),
        (
            WorkManagementOperation::ReopenIssue {
                issue_id: issue.clone(),
                issue_version: v,
                rationale: reason(),
            },
            Some(verified(EvidenceRole::IssueFailedVerification)),
            vec![WorkManagementTarget::Issue(issue.clone(), v)],
            vec![WorkManagementEffect::ReopenIssue(issue)],
        ),
    ];
    assert_eq!(cases.len(), 11);
    for (index, (operation, support, targets, effects)) in cases.into_iter().enumerate() {
        let prepared = WorkManagementPreparedIntent::prepare(
            PreparedIntentId::parse(format!("topology-{index}"))
                .unwrap_or_else(|e| panic!("id: {e}")),
            operation,
            DataClassification::Public,
            support,
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap_or_else(|e| panic!("prepare: {e}"));
        assert_eq!(prepared.preview().targets(), targets);
        assert_eq!(prepared.preview().effects(), effects);
    }
}

#[test]
fn every_new_reason_replacement_and_downstream_field_changes_the_digest() {
    let v = AggregateVersion::initial();
    let d = DecisionId::parse("decision-a").unwrap_or_else(|e| panic!("id: {e}"));
    let replacement = DecisionId::parse("replacement-a").unwrap_or_else(|e| panic!("id: {e}"));
    let downstream = action("downstream-a");
    let rationale = |value: &str| {
        WorkManagementRationale::parse(value).unwrap_or_else(|e| panic!("rationale: {e}"))
    };
    let prepare = |operation, support| {
        WorkManagementPreparedIntent::prepare(
            prepared_id("payload-sensitive"),
            operation,
            DataClassification::Public,
            support,
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap_or_else(|e| panic!("prepare: {e}"))
    };
    let cancel = prepare(
        WorkManagementOperation::CancelAction {
            action_id: action("action-a"),
            action_version: v,
            reason: rationale("Reason A"),
            evidence_classifications: vec![],
        },
        None,
    );
    let cancel_reason = prepare(
        WorkManagementOperation::CancelAction {
            action_id: action("action-a"),
            action_version: v,
            reason: rationale("Reason B"),
            evidence_classifications: vec![],
        },
        None,
    );
    assert_ne!(cancel.payload_digest(), cancel_reason.payload_digest());
    let reopen = prepare(
        WorkManagementOperation::ReopenAction {
            action_id: action("action-a"),
            action_version: v,
            mode: ActionReopenMode::ReopenCompleted,
            reason: rationale("Reason A"),
            evidence_classifications: vec![],
        },
        None,
    );
    let restart = prepare(
        WorkManagementOperation::ReopenAction {
            action_id: action("action-a"),
            action_version: v,
            mode: ActionReopenMode::RestartCancelled,
            reason: rationale("Reason A"),
            evidence_classifications: vec![],
        },
        None,
    );
    assert_ne!(reopen.payload_digest(), restart.payload_digest());
    let supersede = prepare(
        WorkManagementOperation::SupersedeDecision {
            decision_id: d.clone(),
            decision_version: v,
            replacement_decision_id: replacement.clone(),
            replacement_decision_version: v,
            replacement_decision_classification: DataClassification::Public,
            replacement_statement: decision_text("Replacement statement"),
            replacement_rationale: decision_text("Replacement rationale"),
            replacement_impact: decision_text("Replacement impact"),
            replacement_owner: accepted_owner("owner-replacement"),
            replacement_decided_at: UtcTimestamp::from_unix_millis(10_000),
            resulting_action_requests: vec![],
            incomplete_downstream: vec![IncompleteDownstreamWork::Action(
                downstream.clone(),
                v,
                DataClassification::Public,
            )],
        },
        Some(decision_support()),
    );
    for changed in [
        WorkManagementOperation::SupersedeDecision {
            decision_id: d.clone(),
            decision_version: v,
            replacement_decision_id: DecisionId::parse("replacement-b")
                .unwrap_or_else(|e| panic!("id: {e}")),
            replacement_decision_version: v,
            replacement_decision_classification: DataClassification::Public,
            replacement_statement: decision_text("Replacement statement"),
            replacement_rationale: decision_text("Replacement rationale"),
            replacement_impact: decision_text("Replacement impact"),
            replacement_owner: accepted_owner("owner-replacement"),
            replacement_decided_at: UtcTimestamp::from_unix_millis(10_000),
            resulting_action_requests: vec![],
            incomplete_downstream: vec![IncompleteDownstreamWork::Action(
                downstream.clone(),
                v,
                DataClassification::Public,
            )],
        },
        WorkManagementOperation::SupersedeDecision {
            decision_id: d.clone(),
            decision_version: v,
            replacement_decision_id: replacement.clone(),
            replacement_decision_version: v,
            replacement_decision_classification: DataClassification::Public,
            replacement_statement: decision_text("Replacement statement"),
            replacement_rationale: decision_text("Replacement rationale"),
            replacement_impact: decision_text("Replacement impact"),
            replacement_owner: accepted_owner("owner-replacement"),
            replacement_decided_at: UtcTimestamp::from_unix_millis(10_000),
            resulting_action_requests: vec![],
            incomplete_downstream: vec![IncompleteDownstreamWork::Action(
                action("downstream-b"),
                v,
                DataClassification::Public,
            )],
        },
        WorkManagementOperation::SupersedeDecision {
            decision_id: d.clone(),
            decision_version: v,
            replacement_decision_id: replacement.clone(),
            replacement_decision_version: v,
            replacement_decision_classification: DataClassification::Public,
            replacement_statement: decision_text("Replacement statement"),
            replacement_rationale: decision_text("Replacement rationale"),
            replacement_impact: decision_text("Replacement impact"),
            replacement_owner: accepted_owner("owner-replacement"),
            replacement_decided_at: UtcTimestamp::from_unix_millis(10_000),
            resulting_action_requests: vec![],
            incomplete_downstream: vec![IncompleteDownstreamWork::Action(
                downstream.clone(),
                AggregateVersion::new(2).unwrap_or_else(|e| panic!("version: {e}")),
                DataClassification::Public,
            )],
        },
        WorkManagementOperation::SupersedeDecision {
            decision_id: d.clone(),
            decision_version: v,
            replacement_decision_id: replacement.clone(),
            replacement_decision_version: v,
            replacement_decision_classification: DataClassification::Restricted,
            replacement_statement: decision_text("Replacement statement"),
            replacement_rationale: decision_text("Replacement rationale"),
            replacement_impact: decision_text("Replacement impact"),
            replacement_owner: accepted_owner("owner-replacement"),
            replacement_decided_at: UtcTimestamp::from_unix_millis(10_000),
            resulting_action_requests: vec![],
            incomplete_downstream: vec![IncompleteDownstreamWork::Action(
                downstream.clone(),
                v,
                DataClassification::Public,
            )],
        },
        WorkManagementOperation::SupersedeDecision {
            decision_id: d.clone(),
            decision_version: v,
            replacement_decision_id: replacement.clone(),
            replacement_decision_version: v,
            replacement_decision_classification: DataClassification::Public,
            replacement_statement: decision_text("Replacement statement"),
            replacement_rationale: decision_text("Replacement rationale"),
            replacement_impact: decision_text("Replacement impact"),
            replacement_owner: accepted_owner("owner-replacement"),
            replacement_decided_at: UtcTimestamp::from_unix_millis(10_000),
            resulting_action_requests: vec![],
            incomplete_downstream: vec![IncompleteDownstreamWork::Action(
                downstream.clone(),
                v,
                DataClassification::Internal,
            )],
        },
    ] {
        assert_ne!(
            supersede.payload_digest(),
            prepare(changed, Some(decision_support())).payload_digest()
        );
    }
    let risk = RiskId::parse("risk-a").unwrap_or_else(|e| panic!("id: {e}"));
    assert_ne!(
        prepare(
            WorkManagementOperation::CloseRisk {
                risk_id: risk.clone(),
                risk_version: v,
                rationale: rationale("Risk rationale A")
            },
            None
        )
        .payload_digest(),
        prepare(
            WorkManagementOperation::CloseRisk {
                risk_id: risk,
                risk_version: v,
                rationale: rationale("Risk rationale B")
            },
            None
        )
        .payload_digest()
    );
    let verified = || {
        EvidenceOrJudgment::new(
            vec![evidence_for(
                "reopen-verification",
                DataClassification::Public,
                EvidenceRole::IssueFailedVerification,
                EvidenceVerification::Verified {
                    verified_at: UtcTimestamp::from_unix_millis(10),
                    integrity_digest: digest('d'),
                },
            )],
            vec![],
        )
        .unwrap_or_else(|e| panic!("{e}"))
        .evaluate_evidence_required()
        .unwrap_or_else(|e| panic!("{e}"))
    };
    let issue = IssueId::parse("issue-a").unwrap_or_else(|e| panic!("id: {e}"));
    assert_ne!(
        prepare(
            WorkManagementOperation::ReopenIssue {
                issue_id: issue.clone(),
                issue_version: v,
                rationale: rationale("Failed verification A")
            },
            Some(verified())
        )
        .payload_digest(),
        prepare(
            WorkManagementOperation::ReopenIssue {
                issue_id: issue,
                issue_version: v,
                rationale: rationale("Failed verification B")
            },
            Some(verified())
        )
        .payload_digest()
    );
}

#[test]
fn decision_resulting_requests_are_canonical_exact_and_collision_safe() {
    let make = |requests: Vec<DecisionResultingActionRequest>| {
        WorkManagementOperation::ResolveDecisionRequest {
            request_id: DecisionRequestId::parse("decision-request-canonical").unwrap(),
            request_version: AggregateVersion::initial(),
            decision_id: DecisionId::parse("decision-canonical").unwrap(),
            decision_classification: DataClassification::Internal,
            statement: decision_text("Canonical statement"),
            rationale: decision_text("Canonical rationale"),
            impact: decision_text("Canonical impact"),
            decision_owner: accepted_owner("owner-canonical"),
            decided_at: UtcTimestamp::from_unix_millis(20_000),
            resulting_action_requests: requests,
        }
    };
    let prepare = |operation| {
        WorkManagementPreparedIntent::prepare(
            prepared_id("decision-canonical-payload"),
            operation,
            DataClassification::Internal,
            Some(decision_support()),
            UtcTimestamp::from_unix_millis(1_000),
        )
    };
    let a = resulting_request("result-a");
    let b = resulting_request("result-b");
    let forward = prepare(make(vec![a.clone(), b.clone()])).unwrap();
    let reverse = prepare(make(vec![b.clone(), a.clone()])).unwrap();
    assert_eq!(forward.payload_digest(), reverse.payload_digest());
    assert_eq!(
        forward.preview().targets()[2],
        WorkManagementTarget::ActionRequest(a.id.clone(), AggregateVersion::initial())
    );
    assert_eq!(
        forward.preview().targets()[3],
        WorkManagementTarget::ActionRequest(b.id.clone(), AggregateVersion::initial())
    );
    assert_eq!(
        prepare(make(vec![a.clone(), a])),
        Err(PreparedIntentError::InvalidTopology)
    );
    let base = forward;
    type RequestMutation = Box<dyn Fn(&mut DecisionResultingActionRequest)>;
    let mutations: Vec<RequestMutation> = vec![
        Box::new(|item| item.subject = accepted_subject("Changed subject")),
        Box::new(|item| item.details = accepted_commitment("Changed details")),
        Box::new(|item| item.intended_owner = accepted_owner("owner-changed")),
        Box::new(|item| item.due_at = UtcTimestamp::from_unix_millis(20_001)),
        Box::new(|item| item.classification = DataClassification::Restricted),
    ];
    for mutate in mutations {
        let mut changed = b.clone();
        mutate(&mut changed);
        let prepared = prepare(make(vec![resulting_request("result-a"), changed])).unwrap();
        assert_ne!(base.payload_digest(), prepared.payload_digest());
    }
    let changed_id = prepare(make(vec![
        resulting_request("result-a"),
        resulting_request("result-c"),
    ]))
    .unwrap();
    assert_ne!(base.payload_digest(), changed_id.payload_digest());
    let unclassified = DecisionResultingActionRequest {
        classification: DataClassification::Unclassified,
        ..b
    };
    assert_eq!(
        prepare(make(vec![unclassified])),
        Err(PreparedIntentError::UnclassifiedBinding)
    );
}

#[test]
fn every_resolved_and_replacement_decision_record_field_is_digest_bound() {
    let resolve = WorkManagementOperation::ResolveDecisionRequest {
        request_id: DecisionRequestId::parse("digest-request").unwrap(),
        request_version: AggregateVersion::initial(),
        decision_id: DecisionId::parse("digest-decision").unwrap(),
        decision_classification: DataClassification::Internal,
        statement: decision_text("Statement A"),
        rationale: decision_text("Rationale A"),
        impact: decision_text("Impact A"),
        decision_owner: accepted_owner("owner-a"),
        decided_at: UtcTimestamp::from_unix_millis(40_000),
        resulting_action_requests: vec![],
    };
    let digest_for = |id: &str, operation| {
        WorkManagementPreparedIntent::prepare(
            prepared_id(id),
            operation,
            DataClassification::Internal,
            Some(decision_support()),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap()
        .payload_digest()
        .clone()
    };
    let base_digest = digest_for("resolve-field", resolve.clone());
    macro_rules! resolve_change {
        ($field:ident, $value:expr) => {{
            let mut changed = resolve.clone();
            if let WorkManagementOperation::ResolveDecisionRequest { $field, .. } = &mut changed {
                *$field = $value;
            }
            changed
        }};
    }
    for changed in [
        resolve_change!(request_version, AggregateVersion::new(2).unwrap()),
        resolve_change!(decision_id, DecisionId::parse("digest-decision-b").unwrap()),
        resolve_change!(decision_classification, DataClassification::Confidential),
        resolve_change!(statement, decision_text("Statement B")),
        resolve_change!(rationale, decision_text("Rationale B")),
        resolve_change!(impact, decision_text("Impact B")),
        resolve_change!(decision_owner, accepted_owner("owner-b")),
        resolve_change!(decided_at, UtcTimestamp::from_unix_millis(40_001)),
    ] {
        assert_ne!(base_digest, digest_for("resolve-field", changed));
    }

    let replacement = WorkManagementOperation::SupersedeDecision {
        decision_id: DecisionId::parse("supersede-field-original").unwrap(),
        decision_version: AggregateVersion::new(2).unwrap(),
        replacement_decision_id: DecisionId::parse("supersede-field-replacement").unwrap(),
        replacement_decision_version: AggregateVersion::initial(),
        replacement_decision_classification: DataClassification::Internal,
        replacement_statement: decision_text("Replacement statement A"),
        replacement_rationale: decision_text("Replacement rationale A"),
        replacement_impact: decision_text("Replacement impact A"),
        replacement_owner: accepted_owner("replacement-owner-a"),
        replacement_decided_at: UtcTimestamp::from_unix_millis(50_000),
        resulting_action_requests: vec![],
        incomplete_downstream: vec![],
    };
    let replacement_digest = digest_for("replacement-field", replacement.clone());
    macro_rules! replacement_change {
        ($field:ident, $value:expr) => {{
            let mut changed = replacement.clone();
            if let WorkManagementOperation::SupersedeDecision { $field, .. } = &mut changed {
                *$field = $value;
            }
            changed
        }};
    }
    for changed in [
        replacement_change!(decision_version, AggregateVersion::new(3).unwrap()),
        replacement_change!(
            replacement_decision_classification,
            DataClassification::Confidential
        ),
        replacement_change!(
            replacement_statement,
            decision_text("Replacement statement B")
        ),
        replacement_change!(
            replacement_rationale,
            decision_text("Replacement rationale B")
        ),
        replacement_change!(replacement_impact, decision_text("Replacement impact B")),
        replacement_change!(replacement_owner, accepted_owner("replacement-owner-b")),
        replacement_change!(
            replacement_decided_at,
            UtcTimestamp::from_unix_millis(50_001)
        ),
    ] {
        assert_ne!(replacement_digest, digest_for("replacement-field", changed));
    }
}

#[test]
fn resolve_issue_preview_binds_resolution_type_and_rationale_independently() {
    let base = WorkManagementOperation::ResolveIssue {
        issue_id: IssueId::parse("issue-digest").unwrap(),
        issue_version: AggregateVersion::initial(),
        resolution_type: IssueResolutionType::Resolved,
        rationale: WorkManagementRationale::parse("Synthetic resolution rationale".to_owned())
            .unwrap(),
    };
    let digest = |operation| {
        WorkManagementPreparedIntent::prepare(
            prepared_id("prepared-issue-digest"),
            operation,
            DataClassification::Internal,
            Some(issue_resolution_support()),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap()
        .payload_digest()
        .clone()
    };
    let base_digest = digest(base.clone());
    let mut changed_type = base.clone();
    if let WorkManagementOperation::ResolveIssue {
        resolution_type, ..
    } = &mut changed_type
    {
        *resolution_type = IssueResolutionType::Workaround;
    }
    let mut changed_rationale = base;
    if let WorkManagementOperation::ResolveIssue { rationale, .. } = &mut changed_rationale {
        *rationale =
            WorkManagementRationale::parse("Different synthetic rationale".to_owned()).unwrap();
    }
    assert_ne!(base_digest, digest(changed_type));
    assert_ne!(base_digest, digest(changed_rationale));
}

#[test]
fn supersede_contract_requires_support_initial_replacement_and_exact_non_cascading_topology() {
    let downstream_request = ActionRequestId::parse("downstream-request").unwrap();
    let downstream_action = action("downstream-action");
    let operation = |version, requests, downstream| WorkManagementOperation::SupersedeDecision {
        decision_id: DecisionId::parse("decision-original").unwrap(),
        decision_version: AggregateVersion::new(3).unwrap(),
        replacement_decision_id: DecisionId::parse("decision-replacement").unwrap(),
        replacement_decision_version: version,
        replacement_decision_classification: DataClassification::Internal,
        replacement_statement: decision_text("Replacement statement"),
        replacement_rationale: decision_text("Replacement rationale"),
        replacement_impact: decision_text("Replacement impact"),
        replacement_owner: accepted_owner("owner-replacement-contract"),
        replacement_decided_at: UtcTimestamp::from_unix_millis(30_000),
        resulting_action_requests: requests,
        incomplete_downstream: downstream,
    };
    let downstream = vec![
        IncompleteDownstreamWork::ActionRequest(
            downstream_request.clone(),
            AggregateVersion::new(4).unwrap(),
            DataClassification::Internal,
        ),
        IncompleteDownstreamWork::Action(
            downstream_action.clone(),
            AggregateVersion::new(2).unwrap(),
            DataClassification::Confidential,
        ),
    ];
    let prepared = WorkManagementPreparedIntent::prepare(
        prepared_id("supersede-contract"),
        operation(
            AggregateVersion::initial(),
            vec![resulting_request("replacement-request")],
            downstream.clone(),
        ),
        DataClassification::Internal,
        Some(decision_support()),
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap();
    assert!(prepared
        .preview()
        .effects()
        .contains(&WorkManagementEffect::CreateDecision(
            DecisionId::parse("decision-replacement").unwrap()
        )));
    assert!(prepared.preview().effects().contains(
        &WorkManagementEffect::CreateResultingActionRequest(
            ActionRequestId::parse("replacement-request").unwrap()
        )
    ));
    assert!(!prepared.preview().effects().iter().any(|effect| matches!(
        effect,
        WorkManagementEffect::CreateAction(_) | WorkManagementEffect::CancelAction(_)
    )));
    assert_eq!(
        WorkManagementPreparedIntent::prepare(
            prepared_id("supersede-no-support"),
            operation(AggregateVersion::initial(), vec![], vec![]),
            DataClassification::Internal,
            None,
            UtcTimestamp::from_unix_millis(1_000),
        ),
        Err(PreparedIntentError::InvalidSupport)
    );
    let wrong_role = EvidenceOrJudgment::new(
        vec![evidence_for(
            "wrong-role-evidence",
            DataClassification::Internal,
            EvidenceRole::ActionCompletion,
            EvidenceVerification::Verified {
                verified_at: UtcTimestamp::from_unix_millis(10),
                integrity_digest: digest('d'),
            },
        )],
        vec![],
    )
    .unwrap()
    .evaluate_evidence_or_judgment()
    .unwrap();
    assert_eq!(
        WorkManagementPreparedIntent::prepare(
            prepared_id("supersede-wrong-evidence-role"),
            operation(AggregateVersion::initial(), vec![], vec![]),
            DataClassification::Internal,
            Some(wrong_role),
            UtcTimestamp::from_unix_millis(1_000),
        ),
        Err(PreparedIntentError::InvalidSupport)
    );
    assert_eq!(
        WorkManagementPreparedIntent::prepare(
            prepared_id("supersede-existing-replacement"),
            operation(AggregateVersion::new(2).unwrap(), vec![], vec![]),
            DataClassification::Internal,
            Some(decision_support()),
            UtcTimestamp::from_unix_millis(1_000),
        ),
        Err(PreparedIntentError::InvalidTopology)
    );
    for duplicate in [
        vec![
            IncompleteDownstreamWork::ActionRequest(
                ActionRequestId::parse("duplicate-request").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Public,
            ),
            IncompleteDownstreamWork::ActionRequest(
                ActionRequestId::parse("duplicate-request").unwrap(),
                AggregateVersion::new(2).unwrap(),
                DataClassification::Internal,
            ),
        ],
        vec![
            IncompleteDownstreamWork::Action(
                action("duplicate-action"),
                AggregateVersion::initial(),
                DataClassification::Public,
            ),
            IncompleteDownstreamWork::Action(
                action("duplicate-action"),
                AggregateVersion::new(2).unwrap(),
                DataClassification::Internal,
            ),
        ],
    ] {
        assert_eq!(
            WorkManagementPreparedIntent::prepare(
                prepared_id("supersede-duplicate-downstream"),
                operation(AggregateVersion::initial(), vec![], duplicate),
                DataClassification::Internal,
                Some(decision_support()),
                UtcTimestamp::from_unix_millis(1_000),
            ),
            Err(PreparedIntentError::InvalidTopology)
        );
    }
    let typed_same_text = WorkManagementPreparedIntent::prepare(
        prepared_id("supersede-typed-same-text"),
        operation(
            AggregateVersion::initial(),
            vec![],
            vec![
                IncompleteDownstreamWork::ActionRequest(
                    ActionRequestId::parse("same-text").unwrap(),
                    AggregateVersion::initial(),
                    DataClassification::Public,
                ),
                IncompleteDownstreamWork::Action(
                    action("same-text"),
                    AggregateVersion::initial(),
                    DataClassification::Public,
                ),
            ],
        ),
        DataClassification::Internal,
        Some(decision_support()),
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap();
    assert!(typed_same_text.preview().effects().contains(
        &WorkManagementEffect::FlagSupersededPremiseActionRequest(
            ActionRequestId::parse("same-text").unwrap()
        )
    ));
    assert!(typed_same_text.preview().effects().contains(
        &WorkManagementEffect::FlagSupersededPremiseAction(action("same-text"))
    ));
    assert_eq!(
        WorkManagementPreparedIntent::prepare(
            prepared_id("supersede-result-downstream-collision"),
            operation(
                AggregateVersion::initial(),
                vec![resulting_request("collision-request")],
                vec![IncompleteDownstreamWork::ActionRequest(
                    ActionRequestId::parse("collision-request").unwrap(),
                    AggregateVersion::initial(),
                    DataClassification::Public,
                )],
            ),
            DataClassification::Internal,
            Some(decision_support()),
            UtcTimestamp::from_unix_millis(1_000),
        ),
        Err(PreparedIntentError::InvalidTopology)
    );
    assert_eq!(prepared.classification(), DataClassification::Confidential);
}

#[test]
fn digest_binds_immutable_evidence_and_judgment_snapshot() {
    fn make(
        eid: &str,
        time: i64,
        hash: char,
        eclass: DataClassification,
        rationale: &str,
        jclass: DataClassification,
    ) -> WorkManagementPreparedIntent {
        let witness = EvidenceOrJudgment::new(
            vec![evidence(
                eid,
                eclass,
                EvidenceVerification::DegradedLastVerified {
                    last_verified_at: UtcTimestamp::from_unix_millis(time),
                    integrity_digest: digest(hash),
                },
            )],
            vec![judgment(rationale, jclass)],
        )
        .unwrap_or_else(|e| panic!("{e}"))
        .evaluate_evidence_required()
        .unwrap_or_else(|e| panic!("{e}"));
        complete(witness, DataClassification::Public)
    }
    let base = make(
        "evidence-a",
        10,
        'a',
        DataClassification::Internal,
        "Rationale A",
        DataClassification::Internal,
    );
    let changed = [
        make(
            "evidence-b",
            10,
            'a',
            DataClassification::Internal,
            "Rationale A",
            DataClassification::Internal,
        ),
        make(
            "evidence-a",
            11,
            'a',
            DataClassification::Internal,
            "Rationale A",
            DataClassification::Internal,
        ),
        make(
            "evidence-a",
            10,
            'b',
            DataClassification::Internal,
            "Rationale A",
            DataClassification::Internal,
        ),
        make(
            "evidence-a",
            10,
            'a',
            DataClassification::Restricted,
            "Rationale A",
            DataClassification::Internal,
        ),
        make(
            "evidence-a",
            10,
            'a',
            DataClassification::Internal,
            "Rationale B",
            DataClassification::Internal,
        ),
        make(
            "evidence-a",
            10,
            'a',
            DataClassification::Internal,
            "Rationale A",
            DataClassification::Confidential,
        ),
    ];
    assert!(changed
        .iter()
        .all(|item| item.payload_digest() != base.payload_digest()));
}

#[test]
fn approval_is_explicit_without_typed_phrase() {
    let witness = EvidenceOrJudgment::new(
        vec![evidence(
            "verified",
            DataClassification::Public,
            EvidenceVerification::Verified {
                verified_at: UtcTimestamp::from_unix_millis(10),
                integrity_digest: digest('a'),
            },
        )],
        vec![],
    )
    .unwrap_or_else(|e| panic!("{e}"))
    .evaluate_evidence_required()
    .unwrap_or_else(|e| panic!("{e}"));
    let prepared = complete(witness, DataClassification::Public);
    let approval = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("approval-idem").unwrap_or_else(|e| panic!("id: {e}")),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap_or_else(|e| panic!("approval: {e}"));
    assert_eq!(approval.prepared_id(), prepared.id());
    assert_eq!(
        approval.acknowledged_payload_digest(),
        prepared.payload_digest()
    );
    assert_eq!(
        WorkManagementApproval::new(
            prepared.id().clone(),
            AuditActor::HeadOfProducts,
            prepared.payload_digest().clone(),
            IdempotencyId::parse("missing-confirmation").unwrap_or_else(|e| panic!("id: {e}")),
            None,
        ),
        Err(PreparedIntentError::MissingConfirmation)
    );
}
