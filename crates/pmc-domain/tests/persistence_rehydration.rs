use pmc_domain::audit::AuditEventIdSource;
use pmc_domain::classification::DataClassification;
use pmc_domain::delivery::{
    CommandIdentity, CreateInitiative, CreateMilestone, CreateProject, DefinedOutcome,
    DeliveryPersistenceResult, DeliveryPersistenceSnapshot, DeliveryRehydrationError,
    DeliveryReplayCapsule, InMemoryDeliveryService, Initiative, InitiativePersistenceRecord,
    Milestone, MilestonePersistenceRecord, OperationContext, Project, ProjectPersistenceRecord,
    RecordName, UpdateInitiative, UpdateMilestone, UpdateProject, VerificationCriteria,
};
use pmc_domain::identity::{
    AggregateVersion, AuditEventId, CorrelationId, IdempotencyId, InitiativeId, MilestoneId,
    ProjectId,
};
use pmc_domain::provenance::{Provenance, ProvenanceReference};
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::DomainValueError;

#[derive(Clone, Copy)]
struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(1_000)
    }
}

#[derive(Clone, Default)]
struct AuditIds(u64);
impl AuditEventIdSource for AuditIds {
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        self.0 += 1;
        AuditEventId::parse(format!("rehydration-audit-{}", self.0))
    }
}

fn context() -> OperationContext {
    OperationContext {
        correlation_id: CorrelationId::parse("rehydration-correlation").unwrap(),
        idempotency_id: IdempotencyId::parse("rehydration-idempotency").unwrap(),
    }
}

fn create() -> CreateInitiative {
    CreateInitiative {
        context: context(),
        id: InitiativeId::parse("rehydration-initiative").unwrap(),
        name: RecordName::parse("Synthetic initiative", &context().correlation_id).unwrap(),
        defined_outcome: DefinedOutcome::parse(
            "A deterministic persisted outcome",
            &context().correlation_id,
        )
        .unwrap(),
        classification: Some(DataClassification::Internal),
        provenance: Provenance::SyntheticFixture(
            ProvenanceReference::parse("rehydration-fixture").unwrap(),
        ),
    }
}

fn context_for(value: &str) -> OperationContext {
    OperationContext {
        correlation_id: CorrelationId::parse(format!("correlation-{value}")).unwrap(),
        idempotency_id: IdempotencyId::parse(format!("idempotency-{value}")).unwrap(),
    }
}

fn name(value: &str) -> RecordName {
    RecordName::parse(value, &context().correlation_id).unwrap()
}

fn criteria(value: &str) -> VerificationCriteria {
    VerificationCriteria::parse(value, &context().correlation_id).unwrap()
}

fn provenance() -> Provenance {
    Provenance::SyntheticFixture(ProvenanceReference::parse("rehydration-extra-fixture").unwrap())
}

#[test]
fn validated_delivery_snapshot_rehydrates_exact_query_and_replay_state() {
    let mut original = InMemoryDeliveryService::new(FixedClock, AuditIds::default());
    let created = original.create_initiative(create()).unwrap();
    let audit_count = original.audit_events().len();

    let exported = original.persistence_snapshot();
    let snapshot = DeliveryPersistenceSnapshot::validate(
        exported.initiatives().to_vec(),
        exported.projects().to_vec(),
        exported.milestones().to_vec(),
        exported.replay().to_vec(),
        exported.audits().to_vec(),
    )
    .unwrap();
    let mut restored =
        InMemoryDeliveryService::rehydrate(FixedClock, AuditIds::default(), snapshot);

    assert_eq!(restored.initiatives(), vec![created.clone()]);
    assert_eq!(restored.audit_events(), original.audit_events());
    assert_eq!(restored.create_initiative(create()).unwrap(), created);
    assert_eq!(restored.audit_events().len(), audit_count);
}

#[test]
fn duplicate_domain_identity_is_rejected_before_service_construction() {
    let mut service = InMemoryDeliveryService::new(FixedClock, AuditIds::default());
    let initiative = service.create_initiative(create()).unwrap();

    let result = DeliveryPersistenceSnapshot::validate(
        vec![initiative.clone(), initiative],
        vec![],
        vec![],
        vec![],
        vec![],
    );

    assert_eq!(
        result.unwrap_err(),
        DeliveryRehydrationError::DuplicateRecord
    );
}

#[test]
fn all_delivery_command_families_round_trip_with_fan_out_and_exact_replay() {
    let mut service = InMemoryDeliveryService::new(FixedClock, AuditIds::default());
    let initiative = service.create_initiative(create()).unwrap();
    service
        .update_initiative(UpdateInitiative {
            context: context_for("initiative-update"),
            id: initiative.id().clone(),
            expected_version: initiative.version(),
            name: name("Updated initiative"),
            defined_outcome: DefinedOutcome::parse("Updated outcome", &context().correlation_id)
                .unwrap(),
            classification: Some(DataClassification::Restricted),
            provenance: initiative.provenance().clone(),
        })
        .unwrap();
    let project = service
        .create_project(CreateProject {
            context: context_for("project-create"),
            id: ProjectId::parse("rehydration-project").unwrap(),
            name: name("Synthetic project"),
            start_at: UtcTimestamp::from_unix_millis(1),
            end_at: UtcTimestamp::from_unix_millis(2),
            classification: Some(DataClassification::Internal),
            provenance: provenance(),
        })
        .unwrap();
    let milestone = service
        .create_milestone(CreateMilestone {
            context: context_for("milestone-create"),
            id: MilestoneId::parse("rehydration-milestone").unwrap(),
            project_id: project.id().clone(),
            name: name("Synthetic milestone"),
            verification_criteria: criteria("Synthetic evidence"),
            due_at: UtcTimestamp::from_unix_millis(3),
            classification: None,
            provenance: provenance(),
        })
        .unwrap();
    service
        .update_project(UpdateProject {
            context: context_for("project-update"),
            id: project.id().clone(),
            expected_version: project.version(),
            name: name("Updated project"),
            start_at: project.start_at(),
            end_at: project.end_at(),
            classification: Some(DataClassification::Restricted),
            provenance: provenance(),
        })
        .unwrap();
    let inherited = service.inspect_milestone(milestone.id()).unwrap().clone();
    service
        .update_milestone(UpdateMilestone {
            context: context_for("milestone-update"),
            id: inherited.id().clone(),
            expected_version: inherited.version(),
            name: name("Updated milestone"),
            verification_criteria: criteria("Updated evidence"),
            due_at: inherited.due_at(),
            classification: None,
            provenance: provenance(),
        })
        .unwrap();

    let exported = service.persistence_snapshot();
    let replay = exported.replay().to_vec();
    let validated = DeliveryPersistenceSnapshot::validate(
        exported.initiatives().to_vec(),
        exported.projects().to_vec(),
        exported.milestones().to_vec(),
        exported.replay().to_vec(),
        exported.audits().to_vec(),
    )
    .unwrap();
    let mut restored =
        InMemoryDeliveryService::rehydrate(FixedClock, AuditIds::default(), validated);
    assert_eq!(restored.initiatives(), service.initiatives());
    assert_eq!(restored.projects(), service.projects());
    assert_eq!(restored.milestones(), service.milestones());
    assert_eq!(restored.audit_events(), service.audit_events());
    let audit_count = restored.audit_events().len();
    for capsule in &replay {
        replay_delivery_capsule(&mut restored, capsule);
        assert_eq!(restored.audit_events().len(), audit_count);
    }
}

#[test]
fn tampered_payload_missing_audit_and_parent_classification_fail_closed() {
    let mut service = InMemoryDeliveryService::new(FixedClock, AuditIds::default());
    service.create_initiative(create()).unwrap();
    let exported = service.persistence_snapshot();
    let mut replay = exported.replay().to_vec();
    let mut tampered_command = replay[0].command().clone();
    let CommandIdentity::CreateInitiative {
        name: stored_name, ..
    } = &mut tampered_command
    else {
        panic!("synthetic fixture must be an initiative create");
    };
    *stored_name = name("Tampered");
    replay[0] = rebuild_capsule(
        &replay[0],
        tampered_command,
        replay[0].result().clone(),
        replay[0].audit_event_ids().to_vec(),
    );
    assert_eq!(
        DeliveryPersistenceSnapshot::validate(
            exported.initiatives().to_vec(),
            vec![],
            vec![],
            replay,
            exported.audits().to_vec(),
        )
        .unwrap_err(),
        DeliveryRehydrationError::ResultMismatch
    );
    assert_eq!(
        DeliveryPersistenceSnapshot::validate(
            exported.initiatives().to_vec(),
            vec![],
            vec![],
            exported.replay().to_vec(),
            vec![],
        )
        .unwrap_err(),
        DeliveryRehydrationError::AuditMismatch
    );

    let project_id = ProjectId::parse("classified-parent").unwrap();
    let project = Project::rehydrate(ProjectPersistenceRecord {
        id: project_id.clone(),
        name: name("Parent"),
        start_at: UtcTimestamp::from_unix_millis(1),
        end_at: UtcTimestamp::from_unix_millis(2),
        classification: DataClassification::Restricted,
        provenance: provenance(),
        version: AggregateVersion::initial(),
        created_at: UtcTimestamp::from_unix_millis(1),
        updated_at: UtcTimestamp::from_unix_millis(1),
    })
    .unwrap();
    let milestone = Milestone::rehydrate(MilestonePersistenceRecord {
        id: MilestoneId::parse("under-classified-child").unwrap(),
        project_id,
        name: name("Child"),
        verification_criteria: criteria("Evidence"),
        due_at: UtcTimestamp::from_unix_millis(3),
        classification: DataClassification::Public,
        provenance: provenance(),
        version: AggregateVersion::initial(),
        created_at: UtcTimestamp::from_unix_millis(1),
        updated_at: UtcTimestamp::from_unix_millis(1),
    })
    .unwrap();
    assert_eq!(
        DeliveryPersistenceSnapshot::validate(
            vec![],
            vec![project],
            vec![milestone],
            vec![],
            vec![]
        )
        .unwrap_err(),
        DeliveryRehydrationError::ClassificationMismatch
    );
}

fn replay_delivery_capsule(
    service: &mut InMemoryDeliveryService<FixedClock, AuditIds>,
    capsule: &DeliveryReplayCapsule,
) {
    let context = OperationContext {
        correlation_id: CorrelationId::parse("different-replay-correlation").unwrap(),
        idempotency_id: capsule.idempotency_id().clone(),
    };
    match (capsule.command(), capsule.result()) {
        (
            CommandIdentity::CreateInitiative {
                id,
                name,
                defined_outcome,
                classification,
                provenance,
            },
            DeliveryPersistenceResult::Initiative(expected),
        ) => {
            let actual = service
                .create_initiative(CreateInitiative {
                    context,
                    id: id.clone(),
                    name: name.clone(),
                    defined_outcome: defined_outcome.clone(),
                    classification: *classification,
                    provenance: provenance.clone(),
                })
                .unwrap();
            assert_eq!(&actual, expected);
        }
        (
            CommandIdentity::UpdateInitiative {
                id,
                expected_version,
                name,
                defined_outcome,
                classification,
                provenance,
            },
            DeliveryPersistenceResult::Initiative(expected),
        ) => {
            let actual = service
                .update_initiative(UpdateInitiative {
                    context,
                    id: id.clone(),
                    expected_version: *expected_version,
                    name: name.clone(),
                    defined_outcome: defined_outcome.clone(),
                    classification: *classification,
                    provenance: provenance.clone(),
                })
                .unwrap();
            assert_eq!(&actual, expected);
        }
        (
            CommandIdentity::CreateProject {
                id,
                name,
                start_at,
                end_at,
                classification,
                provenance,
            },
            DeliveryPersistenceResult::Project(expected),
        ) => {
            let actual = service
                .create_project(CreateProject {
                    context,
                    id: id.clone(),
                    name: name.clone(),
                    start_at: *start_at,
                    end_at: *end_at,
                    classification: *classification,
                    provenance: provenance.clone(),
                })
                .unwrap();
            assert_eq!(&actual, expected);
        }
        (
            CommandIdentity::UpdateProject {
                id,
                expected_version,
                name,
                start_at,
                end_at,
                classification,
                provenance,
            },
            DeliveryPersistenceResult::Project(expected),
        ) => {
            let actual = service
                .update_project(UpdateProject {
                    context,
                    id: id.clone(),
                    expected_version: *expected_version,
                    name: name.clone(),
                    start_at: *start_at,
                    end_at: *end_at,
                    classification: *classification,
                    provenance: provenance.clone(),
                })
                .unwrap();
            assert_eq!(&actual, expected);
        }
        (
            CommandIdentity::CreateMilestone {
                id,
                project_id,
                name,
                verification_criteria,
                due_at,
                classification,
                provenance,
            },
            DeliveryPersistenceResult::Milestone(expected),
        ) => {
            let actual = service
                .create_milestone(CreateMilestone {
                    context,
                    id: id.clone(),
                    project_id: project_id.clone(),
                    name: name.clone(),
                    verification_criteria: verification_criteria.clone(),
                    due_at: *due_at,
                    classification: *classification,
                    provenance: provenance.clone(),
                })
                .unwrap();
            assert_eq!(&actual, expected);
        }
        (
            CommandIdentity::UpdateMilestone {
                id,
                expected_version,
                name,
                verification_criteria,
                due_at,
                classification,
                provenance,
            },
            DeliveryPersistenceResult::Milestone(expected),
        ) => {
            let actual = service
                .update_milestone(UpdateMilestone {
                    context,
                    id: id.clone(),
                    expected_version: *expected_version,
                    name: name.clone(),
                    verification_criteria: verification_criteria.clone(),
                    due_at: *due_at,
                    classification: *classification,
                    provenance: provenance.clone(),
                })
                .unwrap();
            assert_eq!(&actual, expected);
        }
        _ => panic!("synthetic command and result family must match"),
    }
}

fn rebuild_capsule(
    original: &DeliveryReplayCapsule,
    command: CommandIdentity,
    result: DeliveryPersistenceResult,
    audit_event_ids: Vec<AuditEventId>,
) -> DeliveryReplayCapsule {
    DeliveryReplayCapsule::new(
        original.idempotency_id().clone(),
        command,
        result,
        original.correlation_id().clone(),
        audit_event_ids,
        original.operation_ordinal(),
        original.derived_milestone_mutations().to_vec(),
    )
}

#[test]
fn restricted_to_public_history_is_rejected_even_when_payload_is_self_consistent() {
    let mut service = InMemoryDeliveryService::new(FixedClock, AuditIds::default());
    let created = service
        .create_initiative(CreateInitiative {
            classification: Some(DataClassification::Restricted),
            ..create()
        })
        .unwrap();
    let updated = service
        .update_initiative(UpdateInitiative {
            context: context_for("downgrade-update"),
            id: created.id().clone(),
            expected_version: created.version(),
            name: name("Still restricted"),
            defined_outcome: DefinedOutcome::parse("Still governed", &context().correlation_id)
                .unwrap(),
            classification: Some(DataClassification::Restricted),
            provenance: provenance(),
        })
        .unwrap();
    let exported = service.persistence_snapshot();
    let downgraded = Initiative::rehydrate(InitiativePersistenceRecord {
        id: updated.id().clone(),
        name: name(updated.name()),
        defined_outcome: DefinedOutcome::parse(
            updated.defined_outcome(),
            &context().correlation_id,
        )
        .unwrap(),
        classification: DataClassification::Public,
        provenance: updated.provenance().clone(),
        version: updated.version(),
        created_at: updated.created_at(),
        updated_at: updated.updated_at(),
    })
    .unwrap();
    let mut replay = exported.replay().to_vec();
    let update_index = replay
        .iter()
        .position(|capsule| matches!(capsule.command(), CommandIdentity::UpdateInitiative { .. }))
        .unwrap();
    let mut command = replay[update_index].command().clone();
    let CommandIdentity::UpdateInitiative { classification, .. } = &mut command else {
        unreachable!()
    };
    *classification = Some(DataClassification::Public);
    replay[update_index] = rebuild_capsule(
        &replay[update_index],
        command,
        DeliveryPersistenceResult::Initiative(downgraded.clone()),
        replay[update_index].audit_event_ids().to_vec(),
    );
    assert_eq!(
        DeliveryPersistenceSnapshot::validate(
            vec![downgraded],
            vec![],
            vec![],
            replay,
            exported.audits().to_vec(),
        )
        .unwrap_err(),
        DeliveryRehydrationError::ClassificationMismatch
    );
}

#[test]
fn authoritative_audit_order_rejects_swapped_primaries_and_project_fan_out() {
    let mut independent = InMemoryDeliveryService::new(FixedClock, AuditIds::default());
    independent.create_initiative(create()).unwrap();
    independent
        .create_initiative(CreateInitiative {
            context: context_for("second-primary"),
            id: InitiativeId::parse("second-initiative").unwrap(),
            name: name("Second initiative"),
            defined_outcome: DefinedOutcome::parse("Second outcome", &context().correlation_id)
                .unwrap(),
            classification: Some(DataClassification::Internal),
            provenance: provenance(),
        })
        .unwrap();
    let exported = independent.persistence_snapshot();
    let mut swapped_audits = exported.audits().to_vec();
    swapped_audits.swap(0, 1);
    assert_eq!(
        DeliveryPersistenceSnapshot::validate(
            exported.initiatives().to_vec(),
            vec![],
            vec![],
            exported.replay().to_vec(),
            swapped_audits,
        )
        .unwrap_err(),
        DeliveryRehydrationError::AuditMismatch
    );

    let mut fan_out = InMemoryDeliveryService::new(FixedClock, AuditIds::default());
    let project = fan_out
        .create_project(CreateProject {
            context: context_for("ordered-project-create"),
            id: ProjectId::parse("ordered-project").unwrap(),
            name: name("Ordered project"),
            start_at: UtcTimestamp::from_unix_millis(1),
            end_at: UtcTimestamp::from_unix_millis(2),
            classification: Some(DataClassification::Internal),
            provenance: provenance(),
        })
        .unwrap();
    fan_out
        .create_milestone(CreateMilestone {
            context: context_for("ordered-milestone-create"),
            id: MilestoneId::parse("ordered-milestone").unwrap(),
            project_id: project.id().clone(),
            name: name("Ordered milestone"),
            verification_criteria: criteria("Ordered evidence"),
            due_at: UtcTimestamp::from_unix_millis(3),
            classification: None,
            provenance: provenance(),
        })
        .unwrap();
    fan_out
        .update_project(UpdateProject {
            context: context_for("ordered-project-update"),
            id: project.id().clone(),
            expected_version: project.version(),
            name: name("Raised project"),
            start_at: project.start_at(),
            end_at: project.end_at(),
            classification: Some(DataClassification::Restricted),
            provenance: provenance(),
        })
        .unwrap();
    let exported = fan_out.persistence_snapshot();
    let mut replay = exported.replay().to_vec();
    let project_update = replay
        .iter()
        .position(|value| matches!(value.command(), CommandIdentity::UpdateProject { .. }))
        .unwrap();
    let mut audit_event_ids = replay[project_update].audit_event_ids().to_vec();
    assert_eq!(audit_event_ids.len(), 2);
    audit_event_ids.swap(0, 1);
    replay[project_update] = rebuild_capsule(
        &replay[project_update],
        replay[project_update].command().clone(),
        replay[project_update].result().clone(),
        audit_event_ids,
    );
    assert_eq!(
        DeliveryPersistenceSnapshot::validate(
            exported.initiatives().to_vec(),
            exported.projects().to_vec(),
            exported.milestones().to_vec(),
            replay,
            exported.audits().to_vec(),
        )
        .unwrap_err(),
        DeliveryRehydrationError::AuditMismatch
    );
}

#[test]
fn public_milestone_request_cannot_hide_behind_restricted_result() {
    let mut service = InMemoryDeliveryService::new(FixedClock, AuditIds::default());
    let project = service
        .create_project(CreateProject {
            context: context_for("monotonic-project"),
            id: ProjectId::parse("monotonic-project").unwrap(),
            name: name("Restricted parent"),
            start_at: UtcTimestamp::from_unix_millis(1),
            end_at: UtcTimestamp::from_unix_millis(2),
            classification: Some(DataClassification::Restricted),
            provenance: provenance(),
        })
        .unwrap();
    let milestone = service
        .create_milestone(CreateMilestone {
            context: context_for("monotonic-milestone"),
            id: MilestoneId::parse("monotonic-milestone").unwrap(),
            project_id: project.id().clone(),
            name: name("Restricted child"),
            verification_criteria: criteria("Restricted evidence"),
            due_at: UtcTimestamp::from_unix_millis(3),
            classification: Some(DataClassification::Restricted),
            provenance: provenance(),
        })
        .unwrap();
    service
        .update_milestone(UpdateMilestone {
            context: context_for("monotonic-update"),
            id: milestone.id().clone(),
            expected_version: milestone.version(),
            name: name("Restricted result"),
            verification_criteria: criteria("Still restricted"),
            due_at: milestone.due_at(),
            classification: Some(DataClassification::Restricted),
            provenance: provenance(),
        })
        .unwrap();
    let exported = service.persistence_snapshot();
    let mut replay = exported.replay().to_vec();
    let index = replay
        .iter()
        .position(|value| matches!(value.command(), CommandIdentity::UpdateMilestone { .. }))
        .unwrap();
    let mut command = replay[index].command().clone();
    let CommandIdentity::UpdateMilestone { classification, .. } = &mut command else {
        unreachable!()
    };
    *classification = Some(DataClassification::Public);
    replay[index] = rebuild_capsule(
        &replay[index],
        command,
        replay[index].result().clone(),
        replay[index].audit_event_ids().to_vec(),
    );
    assert_eq!(
        DeliveryPersistenceSnapshot::validate(
            exported.initiatives().to_vec(),
            exported.projects().to_vec(),
            exported.milestones().to_vec(),
            replay,
            exported.audits().to_vec(),
        )
        .unwrap_err(),
        DeliveryRehydrationError::ClassificationMismatch
    );
}
