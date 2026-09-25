//! Delivery family (Initiative/Project/Milestone) H1
//! persistence: create/update for all three record types, the
//! Project-update-cascades-to-child-Milestones rule, and restart recovery
//! through `load_delivery_persistence_snapshot`.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    classification::DataClassification,
    delivery::{
        CreateInitiative, CreateMilestone, CreateProject, DefinedOutcome, OperationContext,
        RecordName, UpdateInitiative, UpdateMilestone, UpdateProject, VerificationCriteria,
    },
    identity::{AuditEventId, CorrelationId, IdempotencyId, InitiativeId, MilestoneId, ProjectId},
    provenance::Provenance,
    time::UtcTimestamp,
};
use pmc_ledger::sqlite::SqliteProductLedger;

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

struct SyntheticLedger(PathBuf);

impl SyntheticLedger {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("synthetic test clock must follow the Unix epoch")
            .as_nanos();
        let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "pmc-synthetic-delivery-repository-{nonce}-{sequence}.sqlite3"
        )))
    }
}

fn context(idempotency: &str, correlation: &str) -> OperationContext {
    OperationContext {
        idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
        correlation_id: CorrelationId::parse(correlation).unwrap(),
    }
}

fn correlation() -> CorrelationId {
    CorrelationId::parse("delivery-test-parse").unwrap()
}

fn create_initiative_command() -> CreateInitiative {
    CreateInitiative {
        id: InitiativeId::parse("synthetic-initiative-1").unwrap(),
        name: RecordName::parse("Synthetic Initiative", &correlation()).unwrap(),
        defined_outcome: DefinedOutcome::parse("Synthetic defined outcome.", &correlation())
            .unwrap(),
        classification: Some(DataClassification::Internal),
        provenance: Provenance::UserEntered,
        context: context("synthetic-initiative-create-1", "synthetic-correlation-1"),
    }
}

fn create_project_command() -> CreateProject {
    CreateProject {
        id: ProjectId::parse("synthetic-project-1").unwrap(),
        name: RecordName::parse("Synthetic Project", &correlation()).unwrap(),
        start_at: UtcTimestamp::from_unix_millis(1_000),
        end_at: UtcTimestamp::from_unix_millis(2_000),
        classification: Some(DataClassification::Internal),
        provenance: Provenance::UserEntered,
        context: context("synthetic-project-create-1", "synthetic-correlation-2"),
    }
}

fn create_milestone_command(project_id: ProjectId) -> CreateMilestone {
    CreateMilestone {
        id: MilestoneId::parse("synthetic-milestone-1").unwrap(),
        project_id,
        name: RecordName::parse("Synthetic Milestone", &correlation()).unwrap(),
        verification_criteria: VerificationCriteria::parse(
            "Synthetic verification criteria.",
            &correlation(),
        )
        .unwrap(),
        due_at: UtcTimestamp::from_unix_millis(1_500),
        classification: None,
        provenance: Provenance::UserEntered,
        context: context("synthetic-milestone-create-1", "synthetic-correlation-3"),
    }
}

#[test]
fn writer_persists_a_created_initiative_and_survives_restart() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    assert_eq!(writer.revision().unwrap(), 0);
    let created = writer
        .create_initiative(
            create_initiative_command(),
            AuditEventId::parse("synthetic-initiative-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    assert_eq!(created.record.name(), "Synthetic Initiative");
    assert_eq!(
        created.record.classification(),
        DataClassification::Internal
    );
    assert_eq!(created.record.version().get(), 1);
    assert_eq!(created.audit_events.len(), 1);
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_delivery_persistence_snapshot().unwrap();
    assert_eq!(snapshot.initiatives().len(), 1);
    assert_eq!(snapshot.initiatives()[0], created.record);
    assert_eq!(snapshot.replay().len(), 1);
    assert_eq!(snapshot.audits().len(), 1);
}

#[test]
fn create_initiative_rejects_a_same_id_replay_with_a_different_payload() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_initiative(
            create_initiative_command(),
            AuditEventId::parse("synthetic-initiative-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let mut drifted = create_initiative_command();
    drifted.name = RecordName::parse("A different name", &correlation()).unwrap();
    let result = writer.create_initiative(
        drifted,
        AuditEventId::parse("synthetic-initiative-audit-2").unwrap(),
        UtcTimestamp::from_unix_millis(2_000),
    );
    assert!(result.is_err());
}

#[test]
fn create_initiative_replay_with_the_same_payload_returns_the_original_outcome() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let first = writer
        .create_initiative(
            create_initiative_command(),
            AuditEventId::parse("synthetic-initiative-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let replayed = writer
        .create_initiative(
            create_initiative_command(),
            AuditEventId::parse("synthetic-initiative-audit-2").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    assert_eq!(replayed.record, first.record);
    assert_eq!(replayed.audit_events[0].id(), first.audit_events[0].id());
}

#[test]
fn writer_updates_a_persisted_initiative_and_survives_restart() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let created = writer
        .create_initiative(
            create_initiative_command(),
            AuditEventId::parse("synthetic-initiative-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let update = UpdateInitiative {
        id: created.record.id().clone(),
        expected_version: created.record.version(),
        name: RecordName::parse("Renamed Initiative", &correlation()).unwrap(),
        defined_outcome: DefinedOutcome::parse("Updated outcome.", &correlation()).unwrap(),
        classification: Some(DataClassification::Confidential),
        provenance: Provenance::UserEntered,
        context: context("synthetic-initiative-update-1", "synthetic-correlation-4"),
    };
    let updated = writer
        .update_initiative(
            update,
            AuditEventId::parse("synthetic-initiative-audit-3").unwrap(),
            UtcTimestamp::from_unix_millis(3_000),
        )
        .unwrap();
    assert_eq!(updated.record.name(), "Renamed Initiative");
    assert_eq!(
        updated.record.classification(),
        DataClassification::Confidential
    );
    assert_eq!(updated.record.version().get(), 2);
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_delivery_persistence_snapshot().unwrap();
    assert_eq!(snapshot.initiatives().len(), 1);
    assert_eq!(snapshot.initiatives()[0], updated.record);
    assert_eq!(snapshot.replay().len(), 2);
}

#[test]
fn update_initiative_rejects_a_stale_expected_version() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let created = writer
        .create_initiative(
            create_initiative_command(),
            AuditEventId::parse("synthetic-initiative-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let update = UpdateInitiative {
        id: created.record.id().clone(),
        expected_version: created.record.version(),
        name: RecordName::parse("First update", &correlation()).unwrap(),
        defined_outcome: DefinedOutcome::parse("First update outcome.", &correlation()).unwrap(),
        classification: None,
        provenance: Provenance::UserEntered,
        context: context("synthetic-initiative-update-1", "synthetic-correlation-4"),
    };
    writer
        .update_initiative(
            update.clone(),
            AuditEventId::parse("synthetic-initiative-audit-3").unwrap(),
            UtcTimestamp::from_unix_millis(3_000),
        )
        .unwrap();
    let mut stale = update;
    stale.name = RecordName::parse("Second update using stale version", &correlation()).unwrap();
    stale.context = context("synthetic-initiative-update-2", "synthetic-correlation-5");
    let result = writer.update_initiative(
        stale,
        AuditEventId::parse("synthetic-initiative-audit-4").unwrap(),
        UtcTimestamp::from_unix_millis(4_000),
    );
    assert!(result.is_err());
}

#[test]
fn update_initiative_rejects_lowering_classification() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let mut initial = create_initiative_command();
    initial.classification = Some(DataClassification::Confidential);
    let created = writer
        .create_initiative(
            initial,
            AuditEventId::parse("synthetic-initiative-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let lowering = UpdateInitiative {
        id: created.record.id().clone(),
        expected_version: created.record.version(),
        name: RecordName::parse(created.record.name(), &correlation()).unwrap(),
        defined_outcome: DefinedOutcome::parse(created.record.defined_outcome(), &correlation())
            .unwrap(),
        classification: Some(DataClassification::Public),
        provenance: Provenance::UserEntered,
        context: context(
            "synthetic-initiative-update-lower-1",
            "synthetic-correlation-6",
        ),
    };
    let result = writer.update_initiative(
        lowering,
        AuditEventId::parse("synthetic-initiative-audit-5").unwrap(),
        UtcTimestamp::from_unix_millis(5_000),
    );
    assert!(result.is_err());
}

#[test]
fn writer_persists_a_created_project_and_survives_restart() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let created = writer
        .create_project(
            create_project_command(),
            AuditEventId::parse("synthetic-project-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    assert_eq!(created.record.name(), "Synthetic Project");
    assert_eq!(created.record.version().get(), 1);
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_delivery_persistence_snapshot().unwrap();
    assert_eq!(snapshot.projects().len(), 1);
    assert_eq!(snapshot.projects()[0], created.record);
}

#[test]
fn create_project_rejects_an_inverted_period() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let mut command = create_project_command();
    command.start_at = UtcTimestamp::from_unix_millis(2_000);
    command.end_at = UtcTimestamp::from_unix_millis(1_000);
    let result = writer.create_project(
        command,
        AuditEventId::parse("synthetic-project-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(1_000),
    );
    assert!(result.is_err());
}

#[test]
fn writer_persists_a_created_milestone_and_survives_restart() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let project = writer
        .create_project(
            create_project_command(),
            AuditEventId::parse("synthetic-project-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let created = writer
        .create_milestone(
            create_milestone_command(project.record.id().clone()),
            AuditEventId::parse("synthetic-milestone-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    assert_eq!(created.record.name(), "Synthetic Milestone");
    assert_eq!(created.record.project_id(), project.record.id());
    // No explicit classification requested: inherits the parent Project's.
    assert_eq!(
        created.record.classification(),
        project.record.classification()
    );
    assert_eq!(created.record.version().get(), 1);
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_delivery_persistence_snapshot().unwrap();
    assert_eq!(snapshot.milestones().len(), 1);
    assert_eq!(snapshot.milestones()[0], created.record);
    assert_eq!(snapshot.replay().len(), 2);
}

#[test]
fn create_milestone_rejects_an_unknown_parent_project() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let result = writer.create_milestone(
        create_milestone_command(ProjectId::parse("unknown-project").unwrap()),
        AuditEventId::parse("synthetic-milestone-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(1_000),
    );
    assert!(result.is_err());
}

#[test]
fn writer_updates_a_persisted_milestone_and_survives_restart() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let project = writer
        .create_project(
            create_project_command(),
            AuditEventId::parse("synthetic-project-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let created = writer
        .create_milestone(
            create_milestone_command(project.record.id().clone()),
            AuditEventId::parse("synthetic-milestone-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    let update = UpdateMilestone {
        id: created.record.id().clone(),
        expected_version: created.record.version(),
        name: RecordName::parse("Renamed Milestone", &correlation()).unwrap(),
        verification_criteria: VerificationCriteria::parse(
            "Updated verification criteria.",
            &correlation(),
        )
        .unwrap(),
        due_at: created.record.due_at(),
        classification: None,
        provenance: Provenance::UserEntered,
        context: context("synthetic-milestone-update-1", "synthetic-correlation-7"),
    };
    let updated = writer
        .update_milestone(
            update,
            AuditEventId::parse("synthetic-milestone-audit-2").unwrap(),
            UtcTimestamp::from_unix_millis(3_000),
        )
        .unwrap();
    assert_eq!(updated.record.name(), "Renamed Milestone");
    assert_eq!(updated.record.version().get(), 2);
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_delivery_persistence_snapshot().unwrap();
    assert_eq!(snapshot.milestones()[0], updated.record);
}

#[test]
fn writer_updates_a_project_without_cascading_when_no_milestone_is_affected() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let project = writer
        .create_project(
            create_project_command(),
            AuditEventId::parse("synthetic-project-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let update = UpdateProject {
        id: project.record.id().clone(),
        expected_version: project.record.version(),
        name: RecordName::parse("Renamed Project", &correlation()).unwrap(),
        start_at: project.record.start_at(),
        end_at: project.record.end_at(),
        classification: None,
        provenance: Provenance::UserEntered,
        context: context("synthetic-project-update-1", "synthetic-correlation-8"),
    };
    let mut remaining = std::collections::VecDeque::new();
    let updated = writer
        .update_project(
            update,
            AuditEventId::parse("synthetic-project-audit-2").unwrap(),
            || {
                remaining
                    .pop_front()
                    .ok_or_else(|| unreachable!("no cascaded milestones expected"))
            },
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    assert_eq!(updated.record.name(), "Renamed Project");
    assert_eq!(updated.audit_events.len(), 1);
    assert!(updated.cascaded_milestones.is_empty());
}

#[test]
fn writer_updates_a_project_and_cascades_to_a_raised_milestone_classification() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let project = writer
        .create_project(
            create_project_command(),
            AuditEventId::parse("synthetic-project-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let milestone = writer
        .create_milestone(
            create_milestone_command(project.record.id().clone()),
            AuditEventId::parse("synthetic-milestone-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    assert_eq!(
        milestone.record.classification(),
        DataClassification::Internal
    );

    let update = UpdateProject {
        id: project.record.id().clone(),
        expected_version: project.record.version(),
        name: RecordName::parse(project.record.name(), &correlation()).unwrap(),
        start_at: project.record.start_at(),
        end_at: project.record.end_at(),
        classification: Some(DataClassification::Confidential),
        provenance: Provenance::UserEntered,
        context: context(
            "synthetic-project-update-raise-1",
            "synthetic-correlation-9",
        ),
    };
    let mut cascade_ids = std::collections::VecDeque::from(vec![AuditEventId::parse(
        "synthetic-milestone-cascade-audit-1",
    )
    .unwrap()]);
    let updated = writer
        .update_project(
            update,
            AuditEventId::parse("synthetic-project-audit-3").unwrap(),
            || {
                cascade_ids
                    .pop_front()
                    .ok_or_else(|| unreachable!("exactly one cascaded milestone expected"))
            },
            UtcTimestamp::from_unix_millis(3_000),
        )
        .unwrap();
    assert_eq!(
        updated.record.classification(),
        DataClassification::Confidential
    );
    assert_eq!(updated.audit_events.len(), 2);
    assert_eq!(updated.cascaded_milestones.len(), 1);
    assert_eq!(
        updated.cascaded_milestones[0].classification(),
        DataClassification::Confidential
    );
    assert_eq!(updated.cascaded_milestones[0].version().get(), 2);
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_delivery_persistence_snapshot().unwrap();
    assert_eq!(snapshot.projects()[0], updated.record);
    assert_eq!(snapshot.milestones()[0], updated.cascaded_milestones[0]);
    assert_eq!(snapshot.replay().len(), 3);
    assert_eq!(snapshot.audits().len(), 4);
}

#[test]
fn update_project_rejects_a_stale_expected_version() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let project = writer
        .create_project(
            create_project_command(),
            AuditEventId::parse("synthetic-project-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let update = UpdateProject {
        id: project.record.id().clone(),
        expected_version: project.record.version(),
        name: RecordName::parse(project.record.name(), &correlation()).unwrap(),
        start_at: project.record.start_at(),
        end_at: project.record.end_at(),
        classification: None,
        provenance: Provenance::UserEntered,
        context: context("synthetic-project-update-1", "synthetic-correlation-8"),
    };
    writer
        .update_project(
            update.clone(),
            AuditEventId::parse("synthetic-project-audit-2").unwrap(),
            || unreachable!("no cascaded milestones expected"),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    let mut stale = update;
    stale.context = context("synthetic-project-update-2", "synthetic-correlation-10");
    let result = writer.update_project(
        stale,
        AuditEventId::parse("synthetic-project-audit-3").unwrap(),
        || unreachable!("no cascaded milestones expected"),
        UtcTimestamp::from_unix_millis(3_000),
    );
    assert!(result.is_err());
}
