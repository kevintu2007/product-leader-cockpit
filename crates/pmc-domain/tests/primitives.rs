use pmc_domain::audit::{
    AuditAction, AuditActor, AuditApprovalOutcome, AuditDisposition, AuditEffectCode,
    AuditEffectScope, AuditEvent, AuditEventCode, AuditExecutionOutcome, AuditModule,
    AuditPolicyOutcome, AuditTarget,
};
use pmc_domain::classification::DataClassification;
use pmc_domain::error::{
    DomainError, ErrorCode, FieldError, MessageKey, MessageParam, PrivateDetailRef,
    SafeErrorExtension, SafeParamValue,
};
use pmc_domain::identity::{
    AggregateVersion, AuditEventId, CorrelationId, IdempotencyId, PortfolioId, ProductId,
    StakeholderId,
};
use pmc_domain::provenance::{Provenance, ProvenanceReference};
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::BoundedText;

#[derive(Clone, Copy)]
struct FixedClock(UtcTimestamp);

impl Clock for FixedClock {
    fn now(&self) -> UtcTimestamp {
        self.0
    }
}

#[test]
fn aggregate_ids_are_typed_bounded_and_path_independent() {
    let portfolio = PortfolioId::parse("portfolio-alpha-01")
        .unwrap_or_else(|error| panic!("valid portfolio id rejected: {error}"));
    let product = ProductId::parse("product-orbit-01")
        .unwrap_or_else(|error| panic!("valid product id rejected: {error}"));

    assert_eq!(portfolio.as_str(), "portfolio-alpha-01");
    assert_eq!(product.as_str(), "product-orbit-01");
    assert!(PortfolioId::parse("").is_err());
    assert!(PortfolioId::parse("../outside").is_err());
    assert!(PortfolioId::parse("contains space").is_err());
    assert!(PortfolioId::parse("a".repeat(65)).is_err());
}

#[test]
fn aggregate_versions_start_nonzero_and_increment_without_wraparound() {
    let initial = AggregateVersion::initial();
    assert_eq!(initial.get(), 1);
    assert_eq!(initial.next().map(AggregateVersion::get), Some(2));
    assert!(AggregateVersion::new(0).is_err());
    let maximum = AggregateVersion::new(u64::MAX)
        .unwrap_or_else(|error| panic!("maximum version rejected: {error}"));
    assert!(maximum.next().is_none());
}

#[test]
fn fixed_clock_returns_the_exact_utc_instant() {
    let timestamp = UtcTimestamp::from_unix_millis(1_908_060_600_000);
    let clock = FixedClock(timestamp);
    assert_eq!(clock.now(), timestamp);
    assert_eq!(timestamp.unix_millis(), 1_908_060_600_000);
}

#[test]
fn classification_combination_is_commutative_and_most_restrictive() {
    use DataClassification::{Confidential, Internal, Public, Restricted};

    for left in [Public, Internal, Confidential, Restricted] {
        for right in [Public, Internal, Confidential, Restricted] {
            assert_eq!(left.combine(right), right.combine(left));
        }
    }
    assert_eq!(Public.combine(Internal), Internal);
    assert_eq!(Internal.combine(Confidential), Confidential);
    assert_eq!(Confidential.combine(Restricted), Restricted);
}

#[test]
fn unclassified_is_the_default_and_dominates_derived_classification() {
    assert_eq!(
        DataClassification::default(),
        DataClassification::Unclassified
    );
    assert_eq!(
        DataClassification::Public.combine(DataClassification::Unclassified),
        DataClassification::Unclassified
    );
    assert_eq!(
        DataClassification::Restricted.combine(DataClassification::Unclassified),
        DataClassification::Unclassified
    );
}

#[test]
fn provenance_is_typed_and_rejects_unbounded_or_private_shaped_references() {
    let reference = ProvenanceReference::parse("synthetic-scenario-v1")
        .unwrap_or_else(|error| panic!("valid provenance rejected: {error}"));
    let provenance = Provenance::SyntheticFixture(reference.clone());

    assert_eq!(provenance.reference(), Some(&reference));
    assert!(ProvenanceReference::parse("").is_err());
    assert!(ProvenanceReference::parse("C:\\private\\record.md").is_err());
    assert!(ProvenanceReference::parse("p".repeat(129)).is_err());
}

#[test]
fn idempotency_and_correlation_ids_reject_ambiguous_values() {
    assert!(IdempotencyId::parse("retry-portfolio-create-01").is_ok());
    assert!(CorrelationId::parse("corr-01HZX9K4J8M7").is_ok());
    assert!(IdempotencyId::parse("retry/id").is_err());
    assert!(CorrelationId::parse("corr id").is_err());
}

#[test]
fn audit_event_carries_safe_identity_time_actor_and_target_values() {
    let timestamp = UtcTimestamp::from_unix_millis(1_908_060_600_000);
    let correlation_id = CorrelationId::parse("corr-audit-01")
        .unwrap_or_else(|error| panic!("correlation id rejected: {error}"));
    let target = AuditTarget::Stakeholder(
        StakeholderId::parse("stakeholder-synthetic-01")
            .unwrap_or_else(|error| panic!("stakeholder id rejected: {error}")),
    );
    let event = AuditEvent::new(
        AuditEventId::parse("audit-event-01")
            .unwrap_or_else(|error| panic!("audit event id rejected: {error}")),
        timestamp,
        AuditActor::HeadOfProducts,
        AuditAction::new(
            AuditModule::Portfolio,
            AuditEventCode::parse("portfolio.stakeholder.created")
                .unwrap_or_else(|error| panic!("event code rejected: {error}")),
            target.clone(),
        ),
        correlation_id.clone(),
        AuditDisposition::new(
            AuditPolicyOutcome::NotRequired,
            AuditApprovalOutcome::NotRequired,
            AuditExecutionOutcome::Succeeded,
            AuditEffectScope::Complete,
            vec![AuditEffectCode::parse("stakeholder.created")
                .unwrap_or_else(|error| panic!("effect code rejected: {error}"))],
        )
        .unwrap_or_else(|error| panic!("valid disposition rejected: {error}")),
    );

    assert_eq!(event.id().as_str(), "audit-event-01");
    assert_eq!(event.occurred_at(), timestamp);
    assert_eq!(event.actor(), AuditActor::HeadOfProducts);
    assert_eq!(event.module(), AuditModule::Portfolio);
    assert_eq!(event.correlation_id(), &correlation_id);
    assert_eq!(event.target(), &target);
    assert_eq!(event.code().as_str(), "portfolio.stakeholder.created");
    assert_eq!(event.policy_outcome(), AuditPolicyOutcome::NotRequired);
    assert_eq!(event.approval_outcome(), AuditApprovalOutcome::NotRequired);
    assert_eq!(event.execution_outcome(), AuditExecutionOutcome::Succeeded);
    assert_eq!(event.actual_effects()[0].as_str(), "stakeholder.created");
    assert!(AuditEventCode::parse("Contains Private Name").is_err());
    assert!(AuditEffectCode::parse("C:\\private\\effect").is_err());
}

#[test]
fn policy_denial_cannot_claim_approval_execution_or_authoritative_effects() {
    let effect = AuditEffectCode::parse("portfolio.created")
        .unwrap_or_else(|error| panic!("effect rejected: {error}"));
    assert!(AuditDisposition::new(
        AuditPolicyOutcome::Denied,
        AuditApprovalOutcome::NotRequired,
        AuditExecutionOutcome::NotAttempted,
        AuditEffectScope::None,
        vec![effect],
    )
    .is_err());
    assert!(AuditDisposition::new(
        AuditPolicyOutcome::Denied,
        AuditApprovalOutcome::Approved,
        AuditExecutionOutcome::Succeeded,
        AuditEffectScope::None,
        Vec::new(),
    )
    .is_err());
}

#[test]
fn rejection_and_not_attempted_execution_cannot_claim_effects() {
    let effect = AuditEffectCode::parse("portfolio.created")
        .unwrap_or_else(|error| panic!("effect rejected: {error}"));
    assert!(AuditDisposition::new(
        AuditPolicyOutcome::Allowed,
        AuditApprovalOutcome::Rejected,
        AuditExecutionOutcome::Succeeded,
        AuditEffectScope::Complete,
        vec![effect.clone()],
    )
    .is_err());
    assert!(AuditDisposition::new(
        AuditPolicyOutcome::Allowed,
        AuditApprovalOutcome::Approved,
        AuditExecutionOutcome::NotAttempted,
        AuditEffectScope::Partial,
        vec![effect],
    )
    .is_err());
}

#[test]
fn cancelled_execution_can_truthfully_record_partial_effects() {
    let disposition = AuditDisposition::new(
        AuditPolicyOutcome::Allowed,
        AuditApprovalOutcome::Approved,
        AuditExecutionOutcome::Cancelled,
        AuditEffectScope::Partial,
        vec![AuditEffectCode::parse("portfolio.created")
            .unwrap_or_else(|error| panic!("effect rejected: {error}"))],
    )
    .unwrap_or_else(|error| panic!("truthful partial cancellation rejected: {error}"));
    assert_eq!(disposition.effect_scope(), AuditEffectScope::Partial);
}

#[test]
fn safe_domain_error_exposes_only_typed_localizable_fields() {
    let secret = "synthetic-private-value-never-echo";
    let error = DomainError::new(
        ErrorCode::ValidationInvalidField,
        MessageKey::parse("portfolio.validation.failed")
            .unwrap_or_else(|parse_error| panic!("message key rejected: {parse_error}")),
        CorrelationId::parse("corr-error-01")
            .unwrap_or_else(|parse_error| panic!("correlation id rejected: {parse_error}")),
        false,
    )
    .with_param(
        MessageParam::new(
            "field",
            SafeParamValue::FieldKey("portfolio.name".to_owned()),
        )
        .unwrap_or_else(|parse_error| panic!("safe param rejected: {parse_error}")),
    )
    .with_extension(SafeErrorExtension::FieldErrors(vec![FieldError::new(
        "portfolio.name",
        "required",
    )
    .unwrap_or_else(|parse_error| {
        panic!("field error rejected: {parse_error}")
    })]))
    .with_private_detail(
        PrivateDetailRef::parse("diag-01HZX9K4J8M7")
            .unwrap_or_else(|parse_error| panic!("detail ref rejected: {parse_error}")),
    );

    let displayed = error.to_string();
    assert_eq!(error.code(), ErrorCode::ValidationInvalidField);
    assert_eq!(error.code().as_str(), "VALIDATION_INVALID_FIELD");
    assert_eq!(error.message_key().as_str(), "portfolio.validation.failed");
    assert_eq!(error.params().len(), 1);
    assert_eq!(error.extensions().len(), 1);
    assert!(!displayed.contains(secret));
    assert!(!displayed.contains("portfolio.name"));
    assert!(!displayed.contains("diag-01HZX9K4J8M7"));
}

#[test]
fn stable_error_codes_use_an_accepted_family_prefix() {
    let codes = [
        ErrorCode::ValidationInvalidField,
        ErrorCode::DomainConflict,
        ErrorCode::DomainNotFound,
        ErrorCode::SecurityPolicyDenied,
        ErrorCode::AiPolicyDenied,
        ErrorCode::SecurityPreviewExpiredOrChanged,
        ErrorCode::DomainIdempotencyConflict,
        ErrorCode::PlatformInternal,
    ];
    let families = [
        "VALIDATION_",
        "DOMAIN_",
        "SECURITY_",
        "LEDGER_",
        "VAULT_",
        "BACKUP_",
        "AI_POLICY_",
        "PLATFORM_",
    ];
    for code in codes {
        assert!(families
            .iter()
            .any(|family| code.as_str().starts_with(family)));
    }
}

#[test]
fn persisted_enums_have_explicit_round_trip_mappings_and_reject_unknown_values() {
    for classification in [
        DataClassification::Public,
        DataClassification::Internal,
        DataClassification::Confidential,
        DataClassification::Restricted,
        DataClassification::Unclassified,
    ] {
        assert_eq!(
            DataClassification::from_persisted(classification.as_persisted()),
            Ok(classification)
        );
    }
    assert!(DataClassification::from_persisted("Public").is_err());

    for actor in [
        AuditActor::HeadOfProducts,
        AuditActor::PolicyAuthorizedSystem,
    ] {
        assert_eq!(AuditActor::from_persisted(actor.as_persisted()), Ok(actor));
    }
    for module in [
        AuditModule::Portfolio,
        AuditModule::WorkManagement,
        AuditModule::Classification,
        AuditModule::Execution,
    ] {
        assert_eq!(
            AuditModule::from_persisted(module.as_persisted()),
            Ok(module)
        );
    }
    assert!(AuditModule::from_persisted("unknown-module").is_err());
    for approval in [
        AuditApprovalOutcome::NotRequired,
        AuditApprovalOutcome::Approved,
        AuditApprovalOutcome::Rejected,
    ] {
        assert_eq!(
            AuditApprovalOutcome::from_persisted(approval.as_persisted()),
            Ok(approval)
        );
    }
    for execution in [
        AuditExecutionOutcome::NotAttempted,
        AuditExecutionOutcome::Succeeded,
        AuditExecutionOutcome::Failed,
        AuditExecutionOutcome::Cancelled,
    ] {
        assert_eq!(
            AuditExecutionOutcome::from_persisted(execution.as_persisted()),
            Ok(execution)
        );
    }
    for scope in [
        AuditEffectScope::None,
        AuditEffectScope::Complete,
        AuditEffectScope::Partial,
    ] {
        assert_eq!(
            AuditEffectScope::from_persisted(scope.as_persisted()),
            Ok(scope)
        );
    }

    for provenance in [
        Provenance::UserEntered,
        Provenance::AuthoritativeTransition(
            ProvenanceReference::parse("transition-01")
                .unwrap_or_else(|error| panic!("reference rejected: {error}")),
        ),
        Provenance::SyntheticFixture(
            ProvenanceReference::parse("fixture-01")
                .unwrap_or_else(|error| panic!("reference rejected: {error}")),
        ),
    ] {
        assert_eq!(
            Provenance::kind_from_persisted(provenance.kind_persisted()),
            Ok(provenance.kind())
        );
    }
    assert!(Provenance::kind_from_persisted("unknown").is_err());
}

#[test]
fn unsafe_message_parameters_are_rejected_before_error_construction() {
    assert!(MessageParam::new(
        "path",
        SafeParamValue::Identifier("C:\\private\\vault".to_owned())
    )
    .is_err());
    assert!(FieldError::new("portfolio.name", "contains private detail").is_err());
    assert!(PrivateDetailRef::parse("C:\\diagnostics\\raw.log").is_err());
}

#[test]
fn bounded_authoritative_text_rejects_blank_oversized_and_control_values() {
    type ShortText = BoundedText<8>;
    assert_eq!(
        ShortText::parse("outcome")
            .unwrap_or_else(|error| panic!("valid bounded text rejected: {error}"))
            .as_str(),
        "outcome"
    );
    assert!(ShortText::parse("   ").is_err());
    assert!(ShortText::parse("123456789").is_err());
    assert!(ShortText::parse("line\nfeed").is_err());
}
