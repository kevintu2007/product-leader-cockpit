use pmc_domain::attention::*;
use pmc_domain::classification::DataClassification;
use pmc_domain::identity::{
    ActionId, ActionRequestId, DecisionRequestId, IssueId, RiskId, StakeholderId,
};
use pmc_domain::issues::IssueDetails;
use pmc_domain::risks::{ResidualExposure, RiskRationale};
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::{
    ActionRequestState, ActionState, DecisionRequestState, IssueState, RiskResponseType, RiskState,
};

fn id<T>(value: &str, parse: fn(String) -> Result<T, pmc_domain::DomainValueError>) -> T {
    match parse(value.to_owned()) {
        Ok(value) => value,
        Err(error) => panic!("invalid synthetic id: {error}"),
    }
}
fn metadata() -> AttentionMetadata {
    AttentionMetadata {
        classification: DataClassification::Public,
        freshness: Freshness::Fresh,
        degraded: false,
    }
}
fn at(value: i64) -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(value)
}
fn exit_proof() -> CompleteRiskExitProof {
    CompleteRiskExitProof::new(
        RiskResponseType::Accept,
        Some(id("stakeholder-1", StakeholderId::parse)),
        Some(
            RiskRationale::parse("accepted with controls".to_owned())
                .unwrap_or_else(|e| panic!("{e}")),
        ),
        Some(
            ResidualExposure::parse("low residual exposure".to_owned())
                .unwrap_or_else(|e| panic!("{e}")),
        ),
        Some(at(100)),
    )
    .unwrap_or_else(|error| panic!("{error:?}"))
}

#[test]
fn composes_flags_in_canonical_order_without_mutating_inputs() {
    let mut input = AttentionInputs {
        as_of: at(100),
        ..Default::default()
    };
    input.thresholds.decision_approaching_deadline_millis = 20;
    input.action_requests.push(ActionRequestAttentionInput {
        id: id("ar-1", ActionRequestId::parse),
        state: ActionRequestState::Open,
        intended_owner_present: false,
        response_due_at: Some(at(100)),
        needs_info: true,
        metadata: AttentionMetadata {
            freshness: Freshness::Stale,
            ..metadata()
        },
    });
    input.actions.push(ActionAttentionInput {
        id: id("a-1", ActionId::parse),
        state: ActionState::InProgress,
        due_at: at(90),
        blocked: true,
        at_risk: false,
        evidence_state: EvidenceAttentionState::Missing,
        superseded_premise: true,
        metadata: metadata(),
    });
    let before = input.clone();
    let result = derive_attention(&input);
    assert_eq!(input, before);
    assert_eq!(result.as_of, at(100));
    assert!(result
        .flags
        .windows(2)
        .all(|pair| (pair[0].target.clone(), pair[0].reason)
            <= (pair[1].target.clone(), pair[1].reason)));
    assert!(result
        .flags
        .iter()
        .any(|flag| flag.reason == AttentionReason::ActionSupersededPremise));
}

#[test]
fn terminal_records_have_no_attention_and_time_is_explicit() {
    let mut input = AttentionInputs {
        as_of: at(101),
        ..Default::default()
    };
    input.action_requests.push(ActionRequestAttentionInput {
        id: id("ar-terminal", ActionRequestId::parse),
        state: ActionRequestState::Accepted,
        intended_owner_present: false,
        response_due_at: Some(at(1)),
        needs_info: true,
        metadata: AttentionMetadata {
            freshness: Freshness::Stale,
            ..metadata()
        },
    });
    input.actions.push(ActionAttentionInput {
        id: id("a-terminal", ActionId::parse),
        state: ActionState::Completed,
        due_at: at(1),
        blocked: true,
        at_risk: true,
        evidence_state: EvidenceAttentionState::VerificationPending,
        superseded_premise: true,
        metadata: metadata(),
    });
    input.decision_requests.push(DecisionRequestAttentionInput {
        id: id("dr-terminal", DecisionRequestId::parse),
        state: DecisionRequestState::Resolved,
        decision_owner_present: false,
        decision_deadline_at: Some(at(1)),
        needs_info: true,
        metadata: AttentionMetadata {
            freshness: Freshness::Stale,
            ..metadata()
        },
    });
    input.risks.push(RiskAttentionInput::in_queue(
        id("risk-terminal", RiskId::parse),
        RiskState::Closed,
        Some(at(1)),
        true,
        true,
        true,
        false,
        metadata(),
    ));
    input.risks.push(RiskAttentionInput::in_queue(
        id("risk-occurred", RiskId::parse),
        RiskState::Occurred,
        Some(at(1)),
        true,
        true,
        true,
        false,
        metadata(),
    ));
    input.issues.push(IssueAttentionInput::closed(
        id("issue-terminal", IssueId::parse),
        AttentionMetadata {
            freshness: Freshness::Stale,
            ..metadata()
        },
    ));
    assert!(derive_attention(&input).flags.is_empty());
}

#[test]
fn deadline_thresholds_are_query_inputs_not_constants() {
    let mut input = AttentionInputs {
        as_of: at(100),
        ..Default::default()
    };
    input.decision_requests.push(DecisionRequestAttentionInput {
        id: id("dr-1", DecisionRequestId::parse),
        state: DecisionRequestState::Open,
        decision_owner_present: true,
        decision_deadline_at: Some(at(140)),
        needs_info: false,
        metadata: metadata(),
    });
    input.thresholds.decision_approaching_deadline_millis = 30;
    assert!(!derive_attention(&input)
        .flags
        .iter()
        .any(|f| f.reason == AttentionReason::DecisionRequestApproachingDeadline));
    input.thresholds.decision_approaching_deadline_millis = 40;
    assert!(derive_attention(&input)
        .flags
        .iter()
        .any(|f| f.reason == AttentionReason::DecisionRequestApproachingDeadline));
}

#[test]
fn every_active_reason_and_metadata_is_derived_from_typed_inputs() {
    let metadata = AttentionMetadata {
        classification: DataClassification::Restricted,
        freshness: Freshness::Stale,
        degraded: true,
    };
    let mut input = AttentionInputs {
        as_of: at(100),
        ..Default::default()
    };
    input.thresholds = AttentionThresholds {
        decision_approaching_deadline_millis: i64::MAX,
        action_at_risk_millis: Some(i64::MAX),
    };
    input.action_requests.push(ActionRequestAttentionInput {
        id: id("ar-all", ActionRequestId::parse),
        state: ActionRequestState::Open,
        intended_owner_present: false,
        response_due_at: Some(at(100)),
        needs_info: true,
        metadata,
    });
    input.actions.push(ActionAttentionInput {
        id: id("a-all", ActionId::parse),
        state: ActionState::InProgress,
        due_at: at(100),
        blocked: true,
        at_risk: false,
        evidence_state: EvidenceAttentionState::VerificationPending,
        superseded_premise: true,
        metadata,
    });
    input.decision_requests.push(DecisionRequestAttentionInput {
        id: id("dr-all", DecisionRequestId::parse),
        state: DecisionRequestState::Open,
        decision_owner_present: false,
        decision_deadline_at: Some(at(100)),
        needs_info: true,
        metadata,
    });
    input.risks.push(
        RiskAttentionInput::exited_after_complete_accept_or_transfer(
            id("risk-all", RiskId::parse),
            RiskState::Open,
            exit_proof(),
            true,
            true,
            true,
            metadata,
        ),
    );
    input.risks.push(RiskAttentionInput::in_queue(
        id("risk-missing-owner", RiskId::parse),
        RiskState::Open,
        None,
        false,
        false,
        false,
        false,
        metadata,
    ));
    input.issues.push(IssueAttentionInput::resolved(
        id("issue-all", IssueId::parse),
        Some(at(100)),
        true,
        ResolvedIssueVerification::EvidenceMissing,
        true,
        metadata,
    ));
    let before = input.clone();
    let result = derive_attention(&input);
    assert_eq!(input, before);
    assert!(result.flags.iter().all(|flag| flag.metadata == metadata));
    assert!(result
        .flags
        .iter()
        .any(|flag| flag.reason == AttentionReason::ActionRequestNeedsInfo));
    assert!(result
        .flags
        .iter()
        .any(|flag| flag.reason == AttentionReason::ActionRequestStale));
    assert!(result
        .flags
        .iter()
        .any(|flag| flag.reason == AttentionReason::ActionRequestMissingIntendedOwner));
    assert!(result
        .flags
        .iter()
        .any(|flag| flag.reason == AttentionReason::ActionRequestResponseDue));
    assert!(result
        .flags
        .iter()
        .any(|flag| flag.reason == AttentionReason::ActionAtRisk));
    assert!(result
        .flags
        .iter()
        .any(|flag| flag.reason == AttentionReason::ActionEvidenceVerificationPending));
    assert!(result
        .flags
        .iter()
        .any(|flag| flag.reason == AttentionReason::DecisionRequestApproachingDeadline));
    assert!(result
        .flags
        .iter()
        .any(|flag| flag.reason == AttentionReason::RiskReviewDue));
    assert!(result
        .flags
        .iter()
        .any(|flag| flag.reason == AttentionReason::RiskExposureIncreased));
    assert!(result
        .flags
        .iter()
        .any(|flag| flag.reason == AttentionReason::RiskEvidenceStale));
    assert!(result
        .flags
        .iter()
        .any(|flag| flag.reason == AttentionReason::RiskControlInvalid));
    assert!(result
        .flags
        .iter()
        .any(|flag| flag.reason == AttentionReason::RiskMissingOwner));
    assert!(result
        .flags
        .iter()
        .any(|flag| flag.reason == AttentionReason::IssueNeedsEvidence));
    assert!(result
        .flags
        .iter()
        .any(|flag| flag.reason == AttentionReason::IssueRecurrence));
    assert!(!result.risk_reentry_summaries.is_empty());
    let later = AttentionInputs {
        as_of: at(101),
        ..input.clone()
    };
    let later_result = derive_attention(&later);
    assert_eq!(input, before);
    assert_ne!(result, later_result);
}

#[test]
fn timestamp_extremes_and_negative_thresholds_are_safe() {
    let mut input = AttentionInputs {
        as_of: at(i64::MAX),
        ..Default::default()
    };
    input.thresholds = AttentionThresholds {
        decision_approaching_deadline_millis: -1,
        action_at_risk_millis: Some(-1),
    };
    input.decision_requests.push(DecisionRequestAttentionInput {
        id: id("dr-extreme", DecisionRequestId::parse),
        state: DecisionRequestState::Open,
        decision_owner_present: true,
        decision_deadline_at: Some(at(i64::MIN)),
        needs_info: false,
        metadata: metadata(),
    });
    input.actions.push(ActionAttentionInput {
        id: id("a-extreme", ActionId::parse),
        state: ActionState::Open,
        due_at: at(i64::MAX),
        blocked: false,
        at_risk: false,
        evidence_state: EvidenceAttentionState::None,
        superseded_premise: false,
        metadata: metadata(),
    });
    let result = derive_attention(&input);
    assert!(result
        .flags
        .iter()
        .any(|flag| flag.reason == AttentionReason::DecisionRequestOverdue));
    assert!(!result
        .flags
        .iter()
        .any(|flag| flag.reason == AttentionReason::DecisionRequestApproachingDeadline));
}

fn reasons_for(result: &AttentionResult, target: AttentionTarget) -> Vec<AttentionReason> {
    result
        .flags
        .iter()
        .filter(|flag| flag.target == target)
        .map(|flag| flag.reason)
        .collect()
}

#[test]
fn risk_exit_and_issue_state_construction_reject_every_invalid_combination() {
    for response in [RiskResponseType::Mitigate, RiskResponseType::Avoid] {
        assert_eq!(
            CompleteRiskExitProof::new(
                response,
                Some(id("owner", StakeholderId::parse)),
                Some(
                    RiskRationale::parse("rationale".to_owned()).unwrap_or_else(|e| panic!("{e}"))
                ),
                Some(
                    ResidualExposure::parse("residual".to_owned())
                        .unwrap_or_else(|e| panic!("{e}"))
                ),
                Some(at(1)),
            ),
            Err(AttentionInputError::RiskExitRequiresAcceptOrTransfer)
        );
    }
    for missing in 0..4 {
        let result = CompleteRiskExitProof::new(
            RiskResponseType::Accept,
            (missing != 0).then(|| id("owner", StakeholderId::parse)),
            (missing != 1).then(|| {
                RiskRationale::parse("rationale".to_owned()).unwrap_or_else(|e| panic!("{e}"))
            }),
            (missing != 2).then(|| {
                ResidualExposure::parse("residual".to_owned()).unwrap_or_else(|e| panic!("{e}"))
            }),
            (missing != 3).then(|| at(1)),
        );
        assert_eq!(result, Err(AttentionInputError::RiskExitProofIncomplete));
    }

    let issue_id = || id("issue-invalid", IssueId::parse);
    for evidence in [
        IssueEvidenceAttentionState::VerificationEvidenceMissing,
        IssueEvidenceAttentionState::VerificationPending,
        IssueEvidenceAttentionState::FailedVerification,
    ] {
        assert_eq!(
            IssueAttentionInput::try_new(
                issue_id(),
                IssueState::Open,
                None,
                false,
                evidence,
                None,
                false,
                metadata()
            ),
            Err(AttentionInputError::OpenIssueCannotCarryVerificationState)
        );
    }
    assert_eq!(
        IssueAttentionInput::try_new(
            issue_id(),
            IssueState::Resolved,
            None,
            false,
            IssueEvidenceAttentionState::ResolutionEvidenceMissing,
            None,
            false,
            metadata()
        ),
        Err(AttentionInputError::ResolvedIssueRequiresResolutionEvidence)
    );
    assert_eq!(
        IssueAttentionInput::try_new(
            issue_id(),
            IssueState::Resolved,
            None,
            false,
            IssueEvidenceAttentionState::FailedVerification,
            None,
            false,
            metadata()
        ),
        Err(AttentionInputError::FailedVerificationRequiresReopenGuidance)
    );
    assert_eq!(
        IssueAttentionInput::try_new(
            issue_id(),
            IssueState::Closed,
            Some(at(1)),
            false,
            IssueEvidenceAttentionState::None,
            None,
            false,
            metadata()
        ),
        Err(AttentionInputError::ClosedIssueCannotRequireAttention)
    );
}

#[test]
fn exact_reason_vectors_cover_deadline_boundaries_thresholds_and_all_reasons() {
    let mut input = AttentionInputs {
        as_of: at(100),
        ..Default::default()
    };
    input.thresholds = AttentionThresholds {
        decision_approaching_deadline_millis: 10,
        action_at_risk_millis: Some(10),
    };
    for (suffix, deadline) in [
        ("before", Some(at(99))),
        ("equal", Some(at(100))),
        ("after", Some(at(101))),
        ("none", None),
    ] {
        input.action_requests.push(ActionRequestAttentionInput {
            id: id(&format!("ar-{suffix}"), ActionRequestId::parse),
            state: ActionRequestState::Open,
            intended_owner_present: suffix != "before",
            response_due_at: deadline,
            needs_info: suffix == "before",
            metadata: if suffix == "before" {
                AttentionMetadata {
                    freshness: Freshness::Stale,
                    ..metadata()
                }
            } else {
                metadata()
            },
        });
        input.decision_requests.push(DecisionRequestAttentionInput {
            id: id(&format!("dr-{suffix}"), DecisionRequestId::parse),
            state: DecisionRequestState::Open,
            decision_owner_present: suffix != "before",
            decision_deadline_at: deadline,
            needs_info: suffix == "before",
            metadata: if suffix == "before" {
                AttentionMetadata {
                    freshness: Freshness::Stale,
                    ..metadata()
                }
            } else {
                metadata()
            },
        });
    }
    for (suffix, due_at, explicit, threshold) in [
        ("overdue", at(99), false, Some(10)),
        ("equal", at(100), false, Some(10)),
        ("edge", at(110), false, Some(10)),
        ("outside", at(111), false, Some(10)),
        ("negative", at(100), false, Some(-1)),
        ("explicit", at(111), true, None),
    ] {
        input.actions.push(ActionAttentionInput {
            id: id(&format!("action-{suffix}"), ActionId::parse),
            state: ActionState::InProgress,
            due_at,
            blocked: suffix == "explicit",
            at_risk: explicit,
            evidence_state: if suffix == "explicit" {
                EvidenceAttentionState::Missing
            } else if suffix == "outside" {
                EvidenceAttentionState::VerificationPending
            } else {
                EvidenceAttentionState::None
            },
            superseded_premise: suffix == "explicit",
            metadata: metadata(),
        });
        if let Some(value) = threshold {
            input.thresholds.action_at_risk_millis = Some(value);
        }
    }
    // Normalize the shared threshold back to ten for exact boundary assertions.
    input.thresholds.action_at_risk_millis = Some(10);
    let result = derive_attention(&input);
    assert_eq!(
        reasons_for(
            &result,
            AttentionTarget::ActionRequest(id("ar-before", ActionRequestId::parse))
        ),
        vec![
            AttentionReason::ActionRequestNeedsInfo,
            AttentionReason::ActionRequestStale,
            AttentionReason::ActionRequestMissingIntendedOwner,
            AttentionReason::ActionRequestResponseOverdue
        ]
    );
    assert_eq!(
        reasons_for(
            &result,
            AttentionTarget::ActionRequest(id("ar-equal", ActionRequestId::parse))
        ),
        vec![AttentionReason::ActionRequestResponseDue]
    );
    assert_eq!(
        reasons_for(
            &result,
            AttentionTarget::ActionRequest(id("ar-after", ActionRequestId::parse))
        ),
        vec![]
    );
    assert_eq!(
        reasons_for(
            &result,
            AttentionTarget::ActionRequest(id("ar-none", ActionRequestId::parse))
        ),
        vec![]
    );
    assert_eq!(
        reasons_for(
            &result,
            AttentionTarget::DecisionRequest(id("dr-before", DecisionRequestId::parse))
        ),
        vec![
            AttentionReason::DecisionRequestNeedsInfo,
            AttentionReason::DecisionRequestMissingDecisionOwner,
            AttentionReason::DecisionRequestOverdue,
            AttentionReason::DecisionRequestStale
        ]
    );
    assert_eq!(
        reasons_for(
            &result,
            AttentionTarget::DecisionRequest(id("dr-equal", DecisionRequestId::parse))
        ),
        vec![AttentionReason::DecisionRequestApproachingDeadline]
    );
    assert_eq!(
        reasons_for(
            &result,
            AttentionTarget::DecisionRequest(id("dr-after", DecisionRequestId::parse))
        ),
        vec![AttentionReason::DecisionRequestApproachingDeadline]
    );
    assert_eq!(
        reasons_for(
            &result,
            AttentionTarget::DecisionRequest(id("dr-none", DecisionRequestId::parse))
        ),
        vec![]
    );
    assert_eq!(
        reasons_for(
            &result,
            AttentionTarget::Action(id("action-overdue", ActionId::parse))
        ),
        vec![AttentionReason::ActionOverdue]
    );
    assert_eq!(
        reasons_for(
            &result,
            AttentionTarget::Action(id("action-equal", ActionId::parse))
        ),
        vec![AttentionReason::ActionAtRisk]
    );
    assert_eq!(
        reasons_for(
            &result,
            AttentionTarget::Action(id("action-edge", ActionId::parse))
        ),
        vec![AttentionReason::ActionAtRisk]
    );
    assert_eq!(
        reasons_for(
            &result,
            AttentionTarget::Action(id("action-outside", ActionId::parse))
        ),
        vec![AttentionReason::ActionEvidenceVerificationPending]
    );
    assert_eq!(
        reasons_for(
            &result,
            AttentionTarget::Action(id("action-explicit", ActionId::parse))
        ),
        vec![
            AttentionReason::ActionBlocked,
            AttentionReason::ActionAtRisk,
            AttentionReason::ActionNeedsEvidence,
            AttentionReason::ActionSupersededPremise
        ]
    );

    let mut negative = input.clone();
    negative.actions.clear();
    negative.thresholds.action_at_risk_millis = Some(-1);
    negative.actions.push(ActionAttentionInput {
        id: id("action-negative-only", ActionId::parse),
        state: ActionState::Open,
        due_at: at(100),
        blocked: false,
        at_risk: false,
        evidence_state: EvidenceAttentionState::None,
        superseded_premise: false,
        metadata: metadata(),
    });
    assert_eq!(
        reasons_for(
            &derive_attention(&negative),
            AttentionTarget::Action(id("action-negative-only", ActionId::parse))
        ),
        vec![AttentionReason::ActionAtRisk]
    );
}

fn failed_guidance() -> FailedVerificationReopenGuidance {
    FailedVerificationReopenGuidance {
        safe_explanation: IssueDetails::parse("verification mismatch".to_owned())
            .unwrap_or_else(|e| panic!("{e}")),
        remediation: IssueDetails::parse("collect corrected evidence".to_owned())
            .unwrap_or_else(|e| panic!("{e}")),
        reopen_rationale: IssueDetails::parse("reopen because closure is unverified".to_owned())
            .unwrap_or_else(|e| panic!("{e}")),
    }
}

#[test]
fn risk_and_issue_vectors_cover_variants_reentry_order_and_reopened_guidance() {
    let degraded = AttentionMetadata {
        classification: DataClassification::Restricted,
        freshness: Freshness::Stale,
        degraded: true,
    };
    let mut input = AttentionInputs {
        as_of: at(100),
        ..Default::default()
    };
    input.risks = vec![
        RiskAttentionInput::exited_after_complete_accept_or_transfer(
            id("risk-b", RiskId::parse),
            RiskState::Open,
            exit_proof(),
            true,
            true,
            true,
            degraded,
        ),
        RiskAttentionInput::exited_after_complete_accept_or_transfer(
            id("risk-a", RiskId::parse),
            RiskState::Open,
            exit_proof(),
            true,
            false,
            false,
            degraded,
        ),
        RiskAttentionInput::in_queue(
            id("risk-queue", RiskId::parse),
            RiskState::Open,
            Some(at(100)),
            false,
            false,
            false,
            false,
            degraded,
        ),
        RiskAttentionInput::in_queue(
            id("risk-none", RiskId::parse),
            RiskState::Open,
            None,
            false,
            false,
            false,
            true,
            degraded,
        ),
        RiskAttentionInput::in_queue(
            id("risk-closed", RiskId::parse),
            RiskState::Closed,
            Some(at(1)),
            true,
            true,
            true,
            false,
            degraded,
        ),
        RiskAttentionInput::in_queue(
            id("risk-occurred", RiskId::parse),
            RiskState::Occurred,
            Some(at(1)),
            true,
            true,
            true,
            false,
            degraded,
        ),
    ];
    let failed = IssueAttentionInput::resolved(
        id("issue-failed", IssueId::parse),
        None,
        false,
        ResolvedIssueVerification::Failed(failed_guidance()),
        false,
        degraded,
    );
    assert_eq!(
        failed.failed_verification_reopen_guidance(),
        Some(&failed_guidance())
    );
    input.issues = vec![
        IssueAttentionInput::open(
            id("issue-open", IssueId::parse),
            Some(at(99)),
            true,
            true,
            true,
            degraded,
        ),
        IssueAttentionInput::resolved(
            id("issue-complete", IssueId::parse),
            Some(at(101)),
            false,
            ResolvedIssueVerification::Complete,
            false,
            metadata(),
        ),
        IssueAttentionInput::resolved(
            id("issue-missing", IssueId::parse),
            Some(at(100)),
            false,
            ResolvedIssueVerification::EvidenceMissing,
            false,
            metadata(),
        ),
        IssueAttentionInput::resolved(
            id("issue-pending", IssueId::parse),
            None,
            false,
            ResolvedIssueVerification::Pending,
            false,
            metadata(),
        ),
        failed,
        IssueAttentionInput::closed(id("issue-closed", IssueId::parse), metadata()),
    ];
    let before = input.clone();
    let result = derive_attention(&input);
    assert_eq!(input, before);
    assert_eq!(
        reasons_for(&result, AttentionTarget::Risk(id("risk-b", RiskId::parse))),
        vec![
            AttentionReason::RiskReviewDue,
            AttentionReason::RiskExposureIncreased,
            AttentionReason::RiskEvidenceStale,
            AttentionReason::RiskControlInvalid
        ]
    );
    assert_eq!(
        reasons_for(&result, AttentionTarget::Risk(id("risk-a", RiskId::parse))),
        vec![
            AttentionReason::RiskReviewDue,
            AttentionReason::RiskExposureIncreased
        ]
    );
    assert_eq!(
        reasons_for(
            &result,
            AttentionTarget::Risk(id("risk-queue", RiskId::parse))
        ),
        vec![
            AttentionReason::RiskReviewDue,
            AttentionReason::RiskMissingOwner
        ]
    );
    assert_eq!(
        reasons_for(
            &result,
            AttentionTarget::Risk(id("risk-none", RiskId::parse))
        ),
        vec![]
    );
    assert_eq!(
        reasons_for(
            &result,
            AttentionTarget::Risk(id("risk-closed", RiskId::parse))
        ),
        vec![]
    );
    assert_eq!(
        reasons_for(
            &result,
            AttentionTarget::Risk(id("risk-occurred", RiskId::parse))
        ),
        vec![]
    );
    assert_eq!(
        result
            .risk_reentry_summaries
            .iter()
            .map(|summary| (summary.target.clone(), summary.reasons.clone()))
            .collect::<Vec<_>>(),
        vec![
            (
                AttentionTarget::Risk(id("risk-a", RiskId::parse)),
                vec![
                    AttentionReason::RiskReviewDue,
                    AttentionReason::RiskExposureIncreased
                ]
            ),
            (
                AttentionTarget::Risk(id("risk-b", RiskId::parse)),
                vec![
                    AttentionReason::RiskReviewDue,
                    AttentionReason::RiskExposureIncreased,
                    AttentionReason::RiskEvidenceStale,
                    AttentionReason::RiskControlInvalid
                ]
            ),
        ]
    );
    assert_eq!(
        reasons_for(
            &result,
            AttentionTarget::Issue(id("issue-open", IssueId::parse))
        ),
        vec![
            AttentionReason::IssueBlocked,
            AttentionReason::IssueResolutionOverdue,
            AttentionReason::IssueNeedsEvidence,
            AttentionReason::IssueStale,
            AttentionReason::IssueRecurrence
        ]
    );
    assert_eq!(
        reasons_for(
            &result,
            AttentionTarget::Issue(id("issue-complete", IssueId::parse))
        ),
        vec![]
    );
    assert_eq!(
        reasons_for(
            &result,
            AttentionTarget::Issue(id("issue-missing", IssueId::parse))
        ),
        vec![
            AttentionReason::IssueResolutionDue,
            AttentionReason::IssueNeedsEvidence
        ]
    );
    assert_eq!(
        reasons_for(
            &result,
            AttentionTarget::Issue(id("issue-pending", IssueId::parse))
        ),
        vec![AttentionReason::IssueNeedsEvidence]
    );
    assert_eq!(
        reasons_for(
            &result,
            AttentionTarget::Issue(id("issue-failed", IssueId::parse))
        ),
        vec![
            AttentionReason::IssueNeedsEvidence,
            AttentionReason::IssueStale
        ]
    );
    let failed_flag = result
        .flags
        .iter()
        .find(|flag| {
            flag.target == AttentionTarget::Issue(id("issue-failed", IssueId::parse))
                && flag.reason == AttentionReason::IssueNeedsEvidence
        })
        .unwrap_or_else(|| panic!("missing failed-verification attention flag"));
    assert_eq!(
        failed_flag.explanation,
        "issue verification failed; reasoned reopening is required"
    );
    assert_eq!(
        failed_flag.failed_verification_guidance,
        Some(failed_guidance())
    );
    assert!(result
        .flags
        .iter()
        .filter(|flag| flag.target != AttentionTarget::Issue(id("issue-failed", IssueId::parse)))
        .all(|flag| flag.failed_verification_guidance.is_none()));
    assert_eq!(
        reasons_for(
            &result,
            AttentionTarget::Issue(id("issue-closed", IssueId::parse))
        ),
        vec![]
    );
    assert!(result
        .flags
        .iter()
        .filter(|flag| flag.metadata == degraded)
        .all(|flag| flag.metadata.degraded));

    let mut permuted = input.clone();
    permuted.risks.reverse();
    permuted.issues.reverse();
    assert_eq!(derive_attention(&permuted), result);
    let later = AttentionInputs {
        as_of: at(101),
        ..input.clone()
    };
    assert_ne!(derive_attention(&later), result);
    assert_eq!(input, before);
}

#[test]
fn every_terminal_variant_is_silent_and_reopened_issue_is_active_again() {
    for (index, state) in [
        ActionRequestState::Accepted,
        ActionRequestState::Declined,
        ActionRequestState::Withdrawn,
    ]
    .into_iter()
    .enumerate()
    {
        let input = AttentionInputs {
            as_of: at(100),
            action_requests: vec![ActionRequestAttentionInput {
                id: id(&format!("ar-terminal-{index}"), ActionRequestId::parse),
                state,
                intended_owner_present: false,
                response_due_at: Some(at(1)),
                needs_info: true,
                metadata: AttentionMetadata {
                    freshness: Freshness::Stale,
                    ..metadata()
                },
            }],
            ..Default::default()
        };
        assert!(derive_attention(&input).flags.is_empty());
    }
    for (index, state) in [ActionState::Completed, ActionState::Cancelled]
        .into_iter()
        .enumerate()
    {
        let input = AttentionInputs {
            as_of: at(100),
            actions: vec![ActionAttentionInput {
                id: id(&format!("action-terminal-{index}"), ActionId::parse),
                state,
                due_at: at(1),
                blocked: true,
                at_risk: true,
                evidence_state: EvidenceAttentionState::Missing,
                superseded_premise: true,
                metadata: metadata(),
            }],
            ..Default::default()
        };
        assert!(derive_attention(&input).flags.is_empty());
    }
    for (index, state) in [
        DecisionRequestState::Resolved,
        DecisionRequestState::Withdrawn,
    ]
    .into_iter()
    .enumerate()
    {
        let input = AttentionInputs {
            as_of: at(100),
            decision_requests: vec![DecisionRequestAttentionInput {
                id: id(
                    &format!("decision-terminal-{index}"),
                    DecisionRequestId::parse,
                ),
                state,
                decision_owner_present: false,
                decision_deadline_at: Some(at(1)),
                needs_info: true,
                metadata: AttentionMetadata {
                    freshness: Freshness::Stale,
                    ..metadata()
                },
            }],
            ..Default::default()
        };
        assert!(derive_attention(&input).flags.is_empty());
    }
    let reopened = AttentionInputs {
        as_of: at(100),
        issues: vec![IssueAttentionInput::open(
            id("issue-reopened", IssueId::parse),
            None,
            false,
            true,
            false,
            metadata(),
        )],
        ..Default::default()
    };
    assert_eq!(
        reasons_for(
            &derive_attention(&reopened),
            AttentionTarget::Issue(id("issue-reopened", IssueId::parse))
        ),
        vec![AttentionReason::IssueNeedsEvidence]
    );
}
