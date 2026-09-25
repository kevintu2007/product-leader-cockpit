use pmc_domain::audit::{AuditActor, AuditEventIdSource};
use pmc_domain::classification::DataClassification;
use pmc_domain::delivery::{
    CreateInitiative, CreateMilestone, CreateProject, DefinedOutcome, InMemoryDeliveryService,
    OperationContext, RecordName, UpdateInitiative, UpdateMilestone, UpdateProject,
    VerificationCriteria,
};
use pmc_domain::error::{ErrorCode, SafeErrorExtension};
use pmc_domain::identity::{
    AggregateVersion, AuditEventId, CorrelationId, IdempotencyId, InitiativeId, MilestoneId,
    ProjectId,
};
use pmc_domain::provenance::{Provenance, ProvenanceReference};
use pmc_domain::time::{Clock, UtcTimestamp};
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

fn service(at: i64) -> InMemoryDeliveryService<FixedClock, SequentialAuditIds> {
    InMemoryDeliveryService::new(
        FixedClock(UtcTimestamp::from_unix_millis(at)),
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

#[test]
fn initiative_requires_a_defined_outcome_and_defaults_to_unclassified() {
    let mut service = service(1000);
    let invalid = DefinedOutcome::parse(" ", &context("i-invalid").correlation_id);
    assert_eq!(
        invalid.unwrap_err().code(),
        ErrorCode::ValidationInvalidField
    );
    assert!(service.audit_events().is_empty());

    let created = service
        .create_initiative(CreateInitiative {
            context: context("i-create"),
            id: InitiativeId::parse("initiative-a").unwrap(),
            name: name("New offer"),
            defined_outcome: outcome("Validate repeatable adoption"),
            classification: None,
            provenance: provenance(),
        })
        .unwrap();
    assert_eq!(created.defined_outcome(), "Validate repeatable adoption");
    assert_eq!(created.classification(), DataClassification::Unclassified);
    assert_eq!(created.created_at().unix_millis(), 1000);
    assert_eq!(created.version(), AggregateVersion::initial());
    assert_eq!(service.audit_events().len(), 1);
}

#[test]
fn project_is_time_bounded_and_not_a_product_alias() {
    let mut service = service(2000);
    let invalid = service.create_project(CreateProject {
        context: context("p-invalid"),
        id: ProjectId::parse("project-a").unwrap(),
        name: name("Launch readiness"),
        start_at: UtcTimestamp::from_unix_millis(20),
        end_at: UtcTimestamp::from_unix_millis(10),
        classification: Some(DataClassification::Internal),
        provenance: provenance(),
    });
    assert_eq!(
        invalid.unwrap_err().code(),
        ErrorCode::ValidationInvalidField
    );

    let project = service
        .create_project(CreateProject {
            context: context("p-create"),
            id: ProjectId::parse("project-a").unwrap(),
            name: name("Launch readiness"),
            start_at: UtcTimestamp::from_unix_millis(10),
            end_at: UtcTimestamp::from_unix_millis(20),
            classification: Some(DataClassification::Internal),
            provenance: provenance(),
        })
        .unwrap();
    assert_eq!(project.start_at().unix_millis(), 10);
    assert_eq!(project.end_at().unix_millis(), 20);
    assert_eq!(project.classification(), DataClassification::Internal);
}

#[test]
fn milestone_is_a_verifiable_checkpoint_without_percentage_progress() {
    let mut service = service(3000);
    service
        .create_project(CreateProject {
            context: context("m-project"),
            id: ProjectId::parse("project-a").unwrap(),
            name: name("Launch readiness"),
            start_at: UtcTimestamp::from_unix_millis(1000),
            end_at: UtcTimestamp::from_unix_millis(5000),
            classification: Some(DataClassification::Confidential),
            provenance: provenance(),
        })
        .unwrap();
    let milestone = service
        .create_milestone(CreateMilestone {
            context: context("m-create"),
            id: MilestoneId::parse("milestone-a").unwrap(),
            project_id: ProjectId::parse("project-a").unwrap(),
            name: name("Readiness review"),
            verification_criteria: criteria("Signed checklist attached"),
            due_at: UtcTimestamp::from_unix_millis(4000),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
        })
        .unwrap();
    assert_eq!(
        milestone.verification_criteria(),
        "Signed checklist attached"
    );
    assert_eq!(milestone.project_id().as_str(), "project-a");
    assert_eq!(milestone.classification(), DataClassification::Confidential);
}

#[test]
fn milestone_requires_an_existing_typed_project_reference() {
    let mut service = service(3000);
    let result = service.create_milestone(CreateMilestone {
        context: context("m-orphan"),
        id: MilestoneId::parse("milestone-orphan").unwrap(),
        project_id: ProjectId::parse("project-missing").unwrap(),
        name: name("Readiness review"),
        verification_criteria: criteria("Signed checklist attached"),
        due_at: UtcTimestamp::from_unix_millis(4000),
        classification: None,
        provenance: provenance(),
    });
    assert_eq!(result.unwrap_err().code(), ErrorCode::DomainNotFound);
    assert!(service.audit_events().is_empty());
}

#[test]
fn expected_version_and_idempotency_are_enforced_without_duplicate_audit() {
    let mut service = service(5000);
    let create = CreateInitiative {
        context: context("retry"),
        id: InitiativeId::parse("initiative-r").unwrap(),
        name: name("Retention"),
        defined_outcome: outcome("Improve retained use"),
        classification: Some(DataClassification::Internal),
        provenance: provenance(),
    };
    let first = service.create_initiative(create.clone()).unwrap();
    let replay = service.create_initiative(create).unwrap();
    assert_eq!(first, replay);
    assert_eq!(service.audit_events().len(), 1);

    let stale = service.update_initiative(UpdateInitiative {
        context: context("stale"),
        id: first.id().clone(),
        expected_version: AggregateVersion::new(2).unwrap(),
        name: name("Retention"),
        defined_outcome: outcome("Improve retained use"),
        classification: None,
        provenance: provenance(),
    });
    assert_eq!(stale.unwrap_err().code(), ErrorCode::DomainConflict);
    assert_eq!(service.audit_events().len(), 1);

    let conflicting_retry = service.create_initiative(CreateInitiative {
        context: context("retry"),
        id: InitiativeId::parse("initiative-other").unwrap(),
        name: name("Other"),
        defined_outcome: outcome("Different"),
        classification: None,
        provenance: provenance(),
    });
    assert_eq!(
        conflicting_retry.unwrap_err().code(),
        ErrorCode::DomainIdempotencyConflict
    );
}

#[test]
fn injected_repository_failure_rolls_back_record_audit_and_idempotency() {
    let mut service = service(6000);
    let command = CreateInitiative {
        context: context("rollback"),
        id: InitiativeId::parse("initiative-rollback").unwrap(),
        name: name("Safe launch"),
        defined_outcome: outcome("Evidence collected"),
        classification: Some(DataClassification::Public),
        provenance: provenance(),
    };
    service.inject_next_commit_failure();
    let failure = service.create_initiative(command.clone()).unwrap_err();
    assert_eq!(failure.code(), ErrorCode::PlatformInternal);
    assert!(failure.retryable());
    assert!(service.inspect_initiative(&command.id).is_none());
    assert!(service.audit_events().is_empty());
    assert!(service.create_initiative(command).is_ok());
    assert_eq!(service.audit_events().len(), 1);
}

#[test]
fn typed_updates_increment_versions_and_preserve_record_identity() {
    let mut service = service(7000);
    let initiative = service
        .create_initiative(CreateInitiative {
            context: context("updates-i-create"),
            id: InitiativeId::parse("initiative-update").unwrap(),
            name: name("Initial initiative"),
            defined_outcome: outcome("Initial outcome"),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
        })
        .unwrap();
    let initiative = service
        .update_initiative(UpdateInitiative {
            context: context("updates-i-update"),
            id: initiative.id().clone(),
            expected_version: initiative.version(),
            name: name("Refined initiative"),
            defined_outcome: outcome("Verified outcome"),
            classification: Some(DataClassification::Internal),
            provenance: provenance(),
        })
        .unwrap();
    assert_eq!(initiative.version().get(), 2);
    assert_eq!(initiative.classification(), DataClassification::Internal);

    let project = service
        .create_project(CreateProject {
            context: context("updates-p-create"),
            id: ProjectId::parse("project-update").unwrap(),
            name: name("Initial project"),
            start_at: UtcTimestamp::from_unix_millis(10),
            end_at: UtcTimestamp::from_unix_millis(20),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
        })
        .unwrap();
    let project = service
        .update_project(UpdateProject {
            context: context("updates-p-update"),
            id: project.id().clone(),
            expected_version: project.version(),
            name: name("Refined project"),
            start_at: UtcTimestamp::from_unix_millis(12),
            end_at: UtcTimestamp::from_unix_millis(24),
            classification: Some(DataClassification::Confidential),
            provenance: provenance(),
        })
        .unwrap();
    assert_eq!(project.version().get(), 2);
    assert_eq!(project.start_at().unix_millis(), 12);

    let milestone = service
        .create_milestone(CreateMilestone {
            context: context("updates-m-create"),
            id: MilestoneId::parse("milestone-update").unwrap(),
            project_id: project.id().clone(),
            name: name("Initial checkpoint"),
            verification_criteria: criteria("Initial evidence"),
            due_at: UtcTimestamp::from_unix_millis(18),
            classification: None,
            provenance: provenance(),
        })
        .unwrap();
    let milestone = service
        .update_milestone(UpdateMilestone {
            context: context("updates-m-update"),
            id: milestone.id().clone(),
            expected_version: milestone.version(),
            name: name("Refined checkpoint"),
            verification_criteria: criteria("Signed evidence attached"),
            due_at: UtcTimestamp::from_unix_millis(19),
            classification: Some(DataClassification::Restricted),
            provenance: provenance(),
        })
        .unwrap();
    assert_eq!(milestone.version().get(), 2);
    assert_eq!(milestone.classification(), DataClassification::Restricted);
    assert_eq!(milestone.project_id(), project.id());
    assert_eq!(service.audit_events().len(), 6);
}

#[test]
fn ordinary_updates_cannot_lower_classification() {
    let mut service = service(8000);
    let created = service
        .create_initiative(CreateInitiative {
            context: context("classification-create"),
            id: InitiativeId::parse("initiative-classified").unwrap(),
            name: name("Sensitive initiative"),
            defined_outcome: outcome("Restricted outcome"),
            classification: Some(DataClassification::Restricted),
            provenance: provenance(),
        })
        .unwrap();
    let result = service.update_initiative(UpdateInitiative {
        context: context("classification-lower"),
        id: created.id().clone(),
        expected_version: created.version(),
        name: name("Sensitive initiative"),
        defined_outcome: outcome("Restricted outcome"),
        classification: Some(DataClassification::Public),
        provenance: provenance(),
    });
    assert_eq!(result.unwrap_err().code(), ErrorCode::SecurityPolicyDenied);
    assert_eq!(
        service
            .inspect_initiative(created.id())
            .unwrap()
            .classification(),
        DataClassification::Restricted
    );
    assert_eq!(service.audit_events().len(), 1);
}

#[test]
fn structured_idempotency_identity_rejects_prior_delimiter_collisions() {
    let mut service = service(9000);
    let first = CreateInitiative {
        context: context("structured-key"),
        id: InitiativeId::parse("initiative-structured").unwrap(),
        name: name("A:B"),
        defined_outcome: outcome("C"),
        classification: Some(DataClassification::Public),
        provenance: provenance(),
    };
    service.create_initiative(first.clone()).unwrap();
    assert!(service.create_initiative(first).is_ok());
    let collision = service.create_initiative(CreateInitiative {
        context: context("structured-key"),
        id: InitiativeId::parse("initiative-structured").unwrap(),
        name: name("A"),
        defined_outcome: outcome("B:C"),
        classification: Some(DataClassification::Public),
        provenance: provenance(),
    });
    assert_eq!(
        collision.unwrap_err().code(),
        ErrorCode::DomainIdempotencyConflict
    );
    assert_eq!(service.audit_events().len(), 1);
}

#[test]
fn bounded_text_rejects_overlong_and_control_values_with_safe_field_errors() {
    let correlation = CorrelationId::parse("correlation-bounded-text").unwrap();
    assert!(RecordName::parse("x".repeat(200), &correlation).is_ok());
    for invalid in ["x".repeat(201), "unsafe\nname".to_owned()] {
        let error = RecordName::parse(invalid, &correlation).unwrap_err();
        assert_eq!(error.code(), ErrorCode::ValidationInvalidField);
        assert!(matches!(
            error.extensions(),
            [SafeErrorExtension::FieldErrors(errors)]
                if errors.len() == 1
                    && errors[0].field_key() == "delivery.name"
                    && errors[0].reason_key() == "delivery.validation.invalid_text"
        ));
    }
}

#[test]
fn audit_identity_and_actor_are_service_owned_and_effect_is_semantic() {
    let mut service = service(10000);
    service
        .create_initiative(CreateInitiative {
            context: context("audit-authority"),
            id: InitiativeId::parse("initiative-audit").unwrap(),
            name: name("Audit authority"),
            defined_outcome: outcome("Authority remains internal"),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
        })
        .unwrap();
    let event = &service.audit_events()[0];
    assert_eq!(event.actor(), AuditActor::HeadOfProducts);
    assert_eq!(event.id().as_str(), "delivery-audit-1");
    assert_eq!(
        event.actual_effects()[0].as_str(),
        "delivery.authoritative-record-changed"
    );
}

#[test]
fn project_classification_raise_atomically_propagates_to_existing_milestones() {
    let mut service = service(11000);
    let project = service
        .create_project(CreateProject {
            context: context("cascade-project"),
            id: ProjectId::parse("project-cascade").unwrap(),
            name: name("Classification cascade"),
            start_at: UtcTimestamp::from_unix_millis(1),
            end_at: UtcTimestamp::from_unix_millis(20),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
        })
        .unwrap();
    let milestone = service
        .create_milestone(CreateMilestone {
            context: context("cascade-milestone"),
            id: MilestoneId::parse("milestone-cascade").unwrap(),
            project_id: project.id().clone(),
            name: name("Cascade checkpoint"),
            verification_criteria: criteria("Classification inherited"),
            due_at: UtcTimestamp::from_unix_millis(15),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
        })
        .unwrap();
    let update = UpdateProject {
        context: context("cascade-update"),
        id: project.id().clone(),
        expected_version: project.version(),
        name: name("Classification cascade"),
        start_at: project.start_at(),
        end_at: project.end_at(),
        classification: Some(DataClassification::Confidential),
        provenance: provenance(),
    };

    service.inject_next_commit_failure();
    assert_eq!(
        service.update_project(update.clone()).unwrap_err().code(),
        ErrorCode::PlatformInternal
    );
    assert_eq!(
        service
            .inspect_project(project.id())
            .unwrap()
            .classification(),
        DataClassification::Public
    );
    assert_eq!(
        service
            .inspect_milestone(milestone.id())
            .unwrap()
            .classification(),
        DataClassification::Public
    );
    assert_eq!(
        service
            .inspect_milestone(milestone.id())
            .unwrap()
            .version()
            .get(),
        1
    );
    assert_eq!(service.audit_events().len(), 2);

    let updated = service.update_project(update.clone()).unwrap();
    assert_eq!(updated.classification(), DataClassification::Confidential);
    assert_eq!(
        service
            .inspect_milestone(milestone.id())
            .unwrap()
            .classification(),
        DataClassification::Confidential
    );
    assert_eq!(
        service
            .inspect_milestone(milestone.id())
            .unwrap()
            .version()
            .get(),
        2
    );
    assert_eq!(service.audit_events().len(), 4);
    assert_eq!(service.update_project(update).unwrap(), updated);
    assert_eq!(service.audit_events().len(), 4);
}

#[test]
fn project_idempotency_conflict_precedes_invalid_period_validation() {
    let mut service = service(12000);
    let created = service
        .create_project(CreateProject {
            context: context("project-create-order"),
            id: ProjectId::parse("project-order").unwrap(),
            name: name("Ordering"),
            start_at: UtcTimestamp::from_unix_millis(1),
            end_at: UtcTimestamp::from_unix_millis(2),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
        })
        .unwrap();
    let changed_invalid_create = service.create_project(CreateProject {
        context: context("project-create-order"),
        id: created.id().clone(),
        name: name("Changed ordering"),
        start_at: UtcTimestamp::from_unix_millis(3),
        end_at: UtcTimestamp::from_unix_millis(2),
        classification: Some(DataClassification::Public),
        provenance: provenance(),
    });
    assert_eq!(
        changed_invalid_create.unwrap_err().code(),
        ErrorCode::DomainIdempotencyConflict
    );

    let update = UpdateProject {
        context: context("project-update-order"),
        id: created.id().clone(),
        expected_version: created.version(),
        name: name("Updated ordering"),
        start_at: UtcTimestamp::from_unix_millis(1),
        end_at: UtcTimestamp::from_unix_millis(3),
        classification: Some(DataClassification::Internal),
        provenance: provenance(),
    };
    service.update_project(update.clone()).unwrap();
    let changed_invalid_update = service.update_project(UpdateProject {
        context: update.context,
        id: update.id,
        expected_version: update.expected_version,
        name: name("Changed update"),
        start_at: UtcTimestamp::from_unix_millis(4),
        end_at: UtcTimestamp::from_unix_millis(3),
        classification: update.classification,
        provenance: update.provenance,
    });
    assert_eq!(
        changed_invalid_update.unwrap_err().code(),
        ErrorCode::DomainIdempotencyConflict
    );
}
