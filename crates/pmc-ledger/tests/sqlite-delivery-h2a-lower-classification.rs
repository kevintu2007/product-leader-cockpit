//! SQLite persistence for the Delivery family's governed
//! "Lower Data Classification" H2a operation across all three record types
//! (Initiative, Project, Milestone). Mirrors
//! `sqlite-portfolio-h2a-lower-classification.rs`'s prepare/execute
//! round-trip and replay coverage, plus a Milestone-specific case exercising
//! the two-level (parent Project + Milestone) synthetic seed that
//! `approve_and_execute_lower_milestone_classification` alone requires.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    audit::AuditActor,
    classification::DataClassification,
    delivery::{
        ApproveAndExecuteLowerInitiativeClassification,
        ApproveAndExecuteLowerMilestoneClassification, ApproveAndExecuteLowerProjectClassification,
        CreateInitiative, CreateMilestone, CreateProject, DefinedOutcome, OperationContext,
        PrepareLowerInitiativeClassification, PrepareLowerMilestoneClassification,
        PrepareLowerProjectClassification, RecordName, UpdateInitiative, UpdateMilestone,
        UpdateProject, VerificationCriteria,
    },
    identity::{
        AggregateVersion, ApprovalReceiptId, AuditEventId, CorrelationId, IdempotencyId,
        InitiativeId, MilestoneId, PreparedIntentId, ProjectId,
    },
    provenance::Provenance,
    time::UtcTimestamp,
    work_management::{
        ApprovalConfirmation, WorkManagementApproval, WorkManagementOperation,
        WorkManagementPreparedIntent, WorkManagementRationale,
    },
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
            "pmc-synthetic-delivery-h2a-lower-classification-{nonce}-{sequence}.sqlite3"
        )))
    }
}

fn initiative_id() -> InitiativeId {
    InitiativeId::parse("synthetic-delivery-h2a-initiative-1").unwrap()
}

fn project_id() -> ProjectId {
    ProjectId::parse("synthetic-delivery-h2a-project-1").unwrap()
}

fn milestone_id() -> MilestoneId {
    MilestoneId::parse("synthetic-delivery-h2a-milestone-1").unwrap()
}

fn context(value: &str) -> OperationContext {
    OperationContext {
        correlation_id: CorrelationId::parse(format!("synthetic-delivery-h2a-correlation-{value}"))
            .unwrap(),
        idempotency_id: IdempotencyId::parse(format!("synthetic-delivery-h2a-idempotency-{value}"))
            .unwrap(),
    }
}

fn name(value: &str) -> RecordName {
    RecordName::parse(value, &context("parse").correlation_id).unwrap()
}

/// Seeds one ledger with an Initiative (Confidential), a Project
/// (Confidential), and a Milestone (Restricted, child of that Project --
/// Restricted still dominates the Project's Confidential floor) so every
/// record type's own lowering test has an independent, correctly-ordered
/// classification to lower from.
fn seeded_ledger() -> (SyntheticLedger, SqliteProductLedger) {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_initiative(
            CreateInitiative {
                context: context("seed-initiative"),
                id: initiative_id(),
                name: name("Initiative"),
                defined_outcome: DefinedOutcome::parse("Outcome", &context("parse").correlation_id)
                    .unwrap(),
                classification: Some(DataClassification::Confidential),
                provenance: Provenance::UserEntered,
            },
            AuditEventId::parse("synthetic-delivery-h2a-audit-seed-initiative").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    writer
        .create_project(
            CreateProject {
                context: context("seed-project"),
                id: project_id(),
                name: name("Project"),
                start_at: UtcTimestamp::from_unix_millis(1),
                end_at: UtcTimestamp::from_unix_millis(2),
                classification: Some(DataClassification::Confidential),
                provenance: Provenance::UserEntered,
            },
            AuditEventId::parse("synthetic-delivery-h2a-audit-seed-project").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    writer
        .create_milestone(
            CreateMilestone {
                context: context("seed-milestone"),
                id: milestone_id(),
                project_id: project_id(),
                name: name("Milestone"),
                verification_criteria: VerificationCriteria::parse(
                    "Evidence",
                    &context("parse").correlation_id,
                )
                .unwrap(),
                due_at: UtcTimestamp::from_unix_millis(3),
                classification: Some(DataClassification::Restricted),
                provenance: Provenance::UserEntered,
            },
            AuditEventId::parse("synthetic-delivery-h2a-audit-seed-milestone").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    (ledger, writer)
}

fn approval_for(
    prepared: &WorkManagementPreparedIntent,
    idempotency_id: &IdempotencyId,
) -> WorkManagementApproval {
    WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        idempotency_id.clone(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap()
}

fn initiative_prepared_intent(
    prepared_id: &str,
    rationale: &WorkManagementRationale,
) -> WorkManagementPreparedIntent {
    WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse(prepared_id).unwrap(),
        WorkManagementOperation::LowerInitiativeClassification {
            initiative_id: initiative_id(),
            initiative_version: AggregateVersion::initial(),
            current_classification: DataClassification::Confidential,
            proposed_classification: DataClassification::Internal,
            rationale: rationale.clone(),
        },
        DataClassification::Confidential,
        None,
        UtcTimestamp::from_unix_millis(100),
    )
    .unwrap()
}

fn project_prepared_intent(
    prepared_id: &str,
    rationale: &WorkManagementRationale,
) -> WorkManagementPreparedIntent {
    WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse(prepared_id).unwrap(),
        WorkManagementOperation::LowerProjectClassification {
            project_id: project_id(),
            project_version: AggregateVersion::initial(),
            current_classification: DataClassification::Confidential,
            proposed_classification: DataClassification::Internal,
            rationale: rationale.clone(),
        },
        DataClassification::Confidential,
        None,
        UtcTimestamp::from_unix_millis(100),
    )
    .unwrap()
}

fn milestone_prepared_intent(
    prepared_id: &str,
    rationale: &WorkManagementRationale,
) -> WorkManagementPreparedIntent {
    WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse(prepared_id).unwrap(),
        WorkManagementOperation::LowerMilestoneClassification {
            milestone_id: milestone_id(),
            milestone_version: AggregateVersion::initial(),
            current_classification: DataClassification::Restricted,
            proposed_classification: DataClassification::Confidential,
            rationale: rationale.clone(),
        },
        DataClassification::Restricted,
        None,
        UtcTimestamp::from_unix_millis(100),
    )
    .unwrap()
}

#[test]
fn prepare_lower_initiative_classification_persists_and_replays_the_exact_preview() {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic lowering rationale").unwrap();
    let prepared =
        initiative_prepared_intent("synthetic-delivery-h2a-prepared-initiative-1", &rationale);
    let command = PrepareLowerInitiativeClassification {
        id: initiative_id(),
        expected_version: AggregateVersion::initial(),
        proposed_classification: DataClassification::Internal,
        rationale: rationale.clone(),
        context: context("prepare-initiative-1"),
    };
    let first = writer
        .prepare_lower_initiative_classification(command.clone(), prepared.clone())
        .unwrap();
    assert_eq!(first, prepared);
    let replay = writer
        .prepare_lower_initiative_classification(command, prepared.clone())
        .unwrap();
    assert_eq!(replay, prepared);
}

#[test]
fn prepare_lower_initiative_classification_rejects_a_non_lowering_proposal() {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Not actually a lowering").unwrap();
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-delivery-h2a-prepared-not-lowering").unwrap(),
        WorkManagementOperation::LowerInitiativeClassification {
            initiative_id: initiative_id(),
            initiative_version: AggregateVersion::initial(),
            current_classification: DataClassification::Confidential,
            proposed_classification: DataClassification::Confidential,
            rationale: rationale.clone(),
        },
        DataClassification::Confidential,
        None,
        UtcTimestamp::from_unix_millis(100),
    )
    .unwrap();
    let command = PrepareLowerInitiativeClassification {
        id: initiative_id(),
        expected_version: AggregateVersion::initial(),
        proposed_classification: DataClassification::Confidential,
        rationale,
        context: context("prepare-initiative-not-lowering"),
    };
    assert!(writer
        .prepare_lower_initiative_classification(command, prepared)
        .is_err());
}

#[test]
fn approve_and_execute_lower_initiative_classification_persists_replays_and_survives_restart() {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic lowering rationale").unwrap();
    let prepared =
        initiative_prepared_intent("synthetic-delivery-h2a-prepared-initiative-2", &rationale);
    writer
        .prepare_lower_initiative_classification(
            PrepareLowerInitiativeClassification {
                id: initiative_id(),
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Internal,
                rationale,
                context: context("prepare-initiative-2"),
            },
            prepared.clone(),
        )
        .unwrap();

    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-delivery-h2a-execute-idempotency-initiative-2").unwrap();
    let approval = approval_for(&prepared, &execute_idempotency_id);
    let execute_command = ApproveAndExecuteLowerInitiativeClassification {
        approval,
        context: OperationContext {
            idempotency_id: execute_idempotency_id,
            correlation_id: CorrelationId::parse(
                "synthetic-delivery-h2a-execute-correlation-initiative-2",
            )
            .unwrap(),
        },
    };
    let audit_event_id =
        AuditEventId::parse("synthetic-delivery-h2a-audit-execute-initiative-2").unwrap();
    let approval_receipt_id =
        ApprovalReceiptId::parse("synthetic-delivery-h2a-receipt-initiative-2").unwrap();
    let outcome = writer
        .approve_and_execute_lower_initiative_classification(
            execute_command.clone(),
            audit_event_id.clone(),
            approval_receipt_id.clone(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    assert_eq!(
        outcome.record.classification(),
        DataClassification::Internal
    );
    assert_eq!(outcome.record.version().get(), 2);

    let replay = writer
        .approve_and_execute_lower_initiative_classification(
            execute_command,
            audit_event_id,
            approval_receipt_id,
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    assert_eq!(replay.record.classification(), DataClassification::Internal);
    assert_eq!(replay.record.version().get(), 2);

    drop(writer);
    let reopened = SqliteProductLedger::open(&_ledger.0).unwrap();
    let snapshot = reopened.load_delivery_persistence_snapshot().unwrap();
    let restored = snapshot
        .initiatives()
        .iter()
        .find(|value| value.id() == &initiative_id())
        .unwrap();
    assert_eq!(restored.classification(), DataClassification::Internal);
    assert_eq!(restored.version().get(), 2);
}

#[test]
fn approve_and_execute_lower_initiative_classification_rejects_a_stale_preview() {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic lowering rationale").unwrap();
    let prepared =
        initiative_prepared_intent("synthetic-delivery-h2a-prepared-initiative-3", &rationale);
    writer
        .prepare_lower_initiative_classification(
            PrepareLowerInitiativeClassification {
                id: initiative_id(),
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Internal,
                rationale,
                context: context("prepare-initiative-3"),
            },
            prepared.clone(),
        )
        .unwrap();

    // Mutate the initiative out from under the prepared preview via an
    // ordinary H1 update that only raises/holds classification (never a
    // lowering, so it is legal through the ordinary path).
    writer
        .update_initiative(
            pmc_domain::delivery::UpdateInitiative {
                id: initiative_id(),
                expected_version: AggregateVersion::initial(),
                name: name("Initiative"),
                defined_outcome: DefinedOutcome::parse("Outcome", &context("parse").correlation_id)
                    .unwrap(),
                classification: None,
                provenance: Provenance::UserEntered,
                context: context("interleaved-update-initiative"),
            },
            AuditEventId::parse("synthetic-delivery-h2a-interleaved-audit-initiative").unwrap(),
            UtcTimestamp::from_unix_millis(150),
        )
        .unwrap();

    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-delivery-h2a-execute-idempotency-initiative-3").unwrap();
    let approval = approval_for(&prepared, &execute_idempotency_id);
    let execute_command = ApproveAndExecuteLowerInitiativeClassification {
        approval,
        context: OperationContext {
            idempotency_id: execute_idempotency_id,
            correlation_id: CorrelationId::parse(
                "synthetic-delivery-h2a-execute-correlation-initiative-3",
            )
            .unwrap(),
        },
    };
    let result = writer.approve_and_execute_lower_initiative_classification(
        execute_command,
        AuditEventId::parse("synthetic-delivery-h2a-audit-execute-initiative-3").unwrap(),
        ApprovalReceiptId::parse("synthetic-delivery-h2a-receipt-initiative-3").unwrap(),
        UtcTimestamp::from_unix_millis(200),
    );
    assert!(result.is_err());
}

#[test]
fn approve_and_execute_lower_project_classification_persists_replays_and_survives_restart() {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic lowering rationale").unwrap();
    let prepared = project_prepared_intent("synthetic-delivery-h2a-prepared-project-1", &rationale);
    writer
        .prepare_lower_project_classification(
            PrepareLowerProjectClassification {
                id: project_id(),
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Internal,
                rationale,
                context: context("prepare-project-1"),
            },
            prepared.clone(),
        )
        .unwrap();

    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-delivery-h2a-execute-idempotency-project-1").unwrap();
    let approval = approval_for(&prepared, &execute_idempotency_id);
    let execute_command = ApproveAndExecuteLowerProjectClassification {
        approval,
        context: OperationContext {
            idempotency_id: execute_idempotency_id,
            correlation_id: CorrelationId::parse(
                "synthetic-delivery-h2a-execute-correlation-project-1",
            )
            .unwrap(),
        },
    };
    let audit_event_id =
        AuditEventId::parse("synthetic-delivery-h2a-audit-execute-project-1").unwrap();
    let approval_receipt_id =
        ApprovalReceiptId::parse("synthetic-delivery-h2a-receipt-project-1").unwrap();
    let outcome = writer
        .approve_and_execute_lower_project_classification(
            execute_command,
            audit_event_id,
            approval_receipt_id,
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    assert_eq!(
        outcome.record.classification(),
        DataClassification::Internal
    );
    assert_eq!(outcome.record.version().get(), 2);

    drop(writer);
    let reopened = SqliteProductLedger::open(&_ledger.0).unwrap();
    let snapshot = reopened.load_delivery_persistence_snapshot().unwrap();
    let restored = snapshot
        .projects()
        .iter()
        .find(|value| value.id() == &project_id())
        .unwrap();
    assert_eq!(restored.classification(), DataClassification::Internal);
    assert_eq!(restored.version().get(), 2);
    // The Milestone's own classification is untouched by lowering its
    // parent Project -- lowering never cascades (only raising does, via
    // `update_project`'s own distinct mechanism).
    let milestone = snapshot
        .milestones()
        .iter()
        .find(|value| value.id() == &milestone_id())
        .unwrap();
    assert_eq!(milestone.classification(), DataClassification::Restricted);
    assert_eq!(milestone.version().get(), 1);
}

#[test]
fn approve_and_execute_lower_project_classification_rejects_a_stale_preview() {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic lowering rationale").unwrap();
    let prepared = project_prepared_intent("synthetic-delivery-h2a-prepared-project-2", &rationale);
    writer
        .prepare_lower_project_classification(
            PrepareLowerProjectClassification {
                id: project_id(),
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Internal,
                rationale,
                context: context("prepare-project-2"),
            },
            prepared.clone(),
        )
        .unwrap();
    writer
        .update_project(
            pmc_domain::delivery::UpdateProject {
                id: project_id(),
                expected_version: AggregateVersion::initial(),
                name: name("Project"),
                start_at: UtcTimestamp::from_unix_millis(1),
                end_at: UtcTimestamp::from_unix_millis(2),
                classification: None,
                provenance: Provenance::UserEntered,
                context: context("interleaved-update-project"),
            },
            AuditEventId::parse("synthetic-delivery-h2a-interleaved-audit-project").unwrap(),
            || unreachable!("no milestone is affected by holding classification steady"),
            UtcTimestamp::from_unix_millis(150),
        )
        .unwrap();

    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-delivery-h2a-execute-idempotency-project-2").unwrap();
    let approval = approval_for(&prepared, &execute_idempotency_id);
    let execute_command = ApproveAndExecuteLowerProjectClassification {
        approval,
        context: OperationContext {
            idempotency_id: execute_idempotency_id,
            correlation_id: CorrelationId::parse(
                "synthetic-delivery-h2a-execute-correlation-project-2",
            )
            .unwrap(),
        },
    };
    let result = writer.approve_and_execute_lower_project_classification(
        execute_command,
        AuditEventId::parse("synthetic-delivery-h2a-audit-execute-project-2").unwrap(),
        ApprovalReceiptId::parse("synthetic-delivery-h2a-receipt-project-2").unwrap(),
        UtcTimestamp::from_unix_millis(200),
    );
    assert!(result.is_err());
}

#[test]
fn approve_and_execute_lower_milestone_classification_persists_replays_and_survives_restart() {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic lowering rationale").unwrap();
    let prepared =
        milestone_prepared_intent("synthetic-delivery-h2a-prepared-milestone-1", &rationale);
    writer
        .prepare_lower_milestone_classification(
            PrepareLowerMilestoneClassification {
                id: milestone_id(),
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Confidential,
                rationale,
                context: context("prepare-milestone-1"),
            },
            prepared.clone(),
        )
        .unwrap();

    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-delivery-h2a-execute-idempotency-milestone-1").unwrap();
    let approval = approval_for(&prepared, &execute_idempotency_id);
    let execute_command = ApproveAndExecuteLowerMilestoneClassification {
        approval,
        context: OperationContext {
            idempotency_id: execute_idempotency_id,
            correlation_id: CorrelationId::parse(
                "synthetic-delivery-h2a-execute-correlation-milestone-1",
            )
            .unwrap(),
        },
    };
    let audit_event_id =
        AuditEventId::parse("synthetic-delivery-h2a-audit-execute-milestone-1").unwrap();
    let approval_receipt_id =
        ApprovalReceiptId::parse("synthetic-delivery-h2a-receipt-milestone-1").unwrap();
    let outcome = writer
        .approve_and_execute_lower_milestone_classification(
            execute_command,
            audit_event_id,
            approval_receipt_id,
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    assert_eq!(
        outcome.record.classification(),
        DataClassification::Confidential
    );
    assert_eq!(outcome.record.version().get(), 2);

    drop(writer);
    let reopened = SqliteProductLedger::open(&_ledger.0).unwrap();
    let snapshot = reopened.load_delivery_persistence_snapshot().unwrap();
    let restored = snapshot
        .milestones()
        .iter()
        .find(|value| value.id() == &milestone_id())
        .unwrap();
    assert_eq!(restored.classification(), DataClassification::Confidential);
    assert_eq!(restored.version().get(), 2);
    // The parent Project is untouched by lowering its child Milestone.
    let project = snapshot
        .projects()
        .iter()
        .find(|value| value.id() == &project_id())
        .unwrap();
    assert_eq!(project.classification(), DataClassification::Confidential);
    assert_eq!(project.version().get(), 1);
}

#[test]
fn approve_and_execute_lower_milestone_classification_rejects_a_stale_preview() {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic lowering rationale").unwrap();
    let prepared =
        milestone_prepared_intent("synthetic-delivery-h2a-prepared-milestone-2", &rationale);
    writer
        .prepare_lower_milestone_classification(
            PrepareLowerMilestoneClassification {
                id: milestone_id(),
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Confidential,
                rationale,
                context: context("prepare-milestone-2"),
            },
            prepared.clone(),
        )
        .unwrap();
    writer
        .update_milestone(
            pmc_domain::delivery::UpdateMilestone {
                id: milestone_id(),
                expected_version: AggregateVersion::initial(),
                name: name("Milestone"),
                verification_criteria: VerificationCriteria::parse(
                    "Evidence",
                    &context("parse").correlation_id,
                )
                .unwrap(),
                due_at: UtcTimestamp::from_unix_millis(3),
                classification: None,
                provenance: Provenance::UserEntered,
                context: context("interleaved-update-milestone"),
            },
            AuditEventId::parse("synthetic-delivery-h2a-interleaved-audit-milestone").unwrap(),
            UtcTimestamp::from_unix_millis(150),
        )
        .unwrap();

    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-delivery-h2a-execute-idempotency-milestone-2").unwrap();
    let approval = approval_for(&prepared, &execute_idempotency_id);
    let execute_command = ApproveAndExecuteLowerMilestoneClassification {
        approval,
        context: OperationContext {
            idempotency_id: execute_idempotency_id,
            correlation_id: CorrelationId::parse(
                "synthetic-delivery-h2a-execute-correlation-milestone-2",
            )
            .unwrap(),
        },
    };
    let result = writer.approve_and_execute_lower_milestone_classification(
        execute_command,
        AuditEventId::parse("synthetic-delivery-h2a-audit-execute-milestone-2").unwrap(),
        ApprovalReceiptId::parse("synthetic-delivery-h2a-receipt-milestone-2").unwrap(),
        UtcTimestamp::from_unix_millis(200),
    );
    assert!(result.is_err());
}

// Regression coverage added after an independent fresh-context review
// found that `approve_and_execute_lower_*_classification`'s
// execute-time persistence read the target's *current* durable row instead
// of storing an execute-time snapshot -- silently correct only as long as
// nothing else ever touched the record afterward. The three tests below
// each chain a lowering execute with a *later*, ordinary H1 update, which
// the original bug could not survive (V35 migration + delivery_repository.rs
// fix). A fourth test confirms the live idempotent-replay path returns the
// exact original lowering outcome, not whatever the record looks like now.
// A fifth confirms the replay path rejects an approval whose own
// `idempotency_id` does not match the calling context's, closing the
// idempotency-binding gap the same review found.

#[test]
fn approve_and_execute_lower_initiative_classification_survives_a_later_h1_update_and_restart() {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic lowering rationale").unwrap();
    let prepared =
        initiative_prepared_intent("synthetic-delivery-h2a-prepared-initiative-4", &rationale);
    writer
        .prepare_lower_initiative_classification(
            PrepareLowerInitiativeClassification {
                id: initiative_id(),
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Internal,
                rationale,
                context: context("prepare-initiative-4"),
            },
            prepared.clone(),
        )
        .unwrap();
    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-delivery-h2a-execute-idempotency-initiative-4").unwrap();
    let approval = approval_for(&prepared, &execute_idempotency_id);
    writer
        .approve_and_execute_lower_initiative_classification(
            ApproveAndExecuteLowerInitiativeClassification {
                approval,
                context: OperationContext {
                    idempotency_id: execute_idempotency_id,
                    correlation_id: CorrelationId::parse(
                        "synthetic-delivery-h2a-execute-correlation-initiative-4",
                    )
                    .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-delivery-h2a-audit-execute-initiative-4").unwrap(),
            ApprovalReceiptId::parse("synthetic-delivery-h2a-receipt-initiative-4").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();

    // The bug: a later, ordinary H1 update after the lowering used to
    // corrupt both live replay and restart decode of the earlier lowering
    // capsule, because it was reconstructed from "current" state instead of
    // an execute-time snapshot.
    writer
        .update_initiative(
            UpdateInitiative {
                id: initiative_id(),
                expected_version: AggregateVersion::new(2).unwrap(),
                name: name("Initiative renamed after lowering"),
                defined_outcome: DefinedOutcome::parse("Outcome", &context("parse").correlation_id)
                    .unwrap(),
                classification: None,
                provenance: Provenance::UserEntered,
                context: context("post-lowering-update-initiative"),
            },
            AuditEventId::parse("synthetic-delivery-h2a-post-lowering-audit-initiative").unwrap(),
            UtcTimestamp::from_unix_millis(300),
        )
        .unwrap();

    drop(writer);
    let reopened = SqliteProductLedger::open(&_ledger.0).unwrap();
    let snapshot = reopened.load_delivery_persistence_snapshot().expect(
        "restart decode must succeed even after a lowering is followed by an ordinary update",
    );
    let restored = snapshot
        .initiatives()
        .iter()
        .find(|value| value.id() == &initiative_id())
        .unwrap();
    assert_eq!(restored.classification(), DataClassification::Internal);
    assert_eq!(restored.version().get(), 3);
    assert_eq!(restored.name(), "Initiative renamed after lowering");
}

#[test]
fn approve_and_execute_lower_project_classification_survives_a_later_h1_update_and_restart() {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic lowering rationale").unwrap();
    let prepared = project_prepared_intent("synthetic-delivery-h2a-prepared-project-3", &rationale);
    writer
        .prepare_lower_project_classification(
            PrepareLowerProjectClassification {
                id: project_id(),
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Internal,
                rationale,
                context: context("prepare-project-3"),
            },
            prepared.clone(),
        )
        .unwrap();
    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-delivery-h2a-execute-idempotency-project-3").unwrap();
    let approval = approval_for(&prepared, &execute_idempotency_id);
    writer
        .approve_and_execute_lower_project_classification(
            ApproveAndExecuteLowerProjectClassification {
                approval,
                context: OperationContext {
                    idempotency_id: execute_idempotency_id,
                    correlation_id: CorrelationId::parse(
                        "synthetic-delivery-h2a-execute-correlation-project-3",
                    )
                    .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-delivery-h2a-audit-execute-project-3").unwrap(),
            ApprovalReceiptId::parse("synthetic-delivery-h2a-receipt-project-3").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();

    writer
        .update_project(
            UpdateProject {
                id: project_id(),
                expected_version: AggregateVersion::new(2).unwrap(),
                name: name("Project renamed after lowering"),
                start_at: UtcTimestamp::from_unix_millis(1),
                end_at: UtcTimestamp::from_unix_millis(2),
                classification: None,
                provenance: Provenance::UserEntered,
                context: context("post-lowering-update-project"),
            },
            AuditEventId::parse("synthetic-delivery-h2a-post-lowering-audit-project").unwrap(),
            || unreachable!("no milestone is affected by holding classification steady"),
            UtcTimestamp::from_unix_millis(300),
        )
        .unwrap();

    drop(writer);
    let reopened = SqliteProductLedger::open(&_ledger.0).unwrap();
    let snapshot = reopened.load_delivery_persistence_snapshot().expect(
        "restart decode must succeed even after a lowering is followed by an ordinary update",
    );
    let restored = snapshot
        .projects()
        .iter()
        .find(|value| value.id() == &project_id())
        .unwrap();
    assert_eq!(restored.classification(), DataClassification::Internal);
    assert_eq!(restored.version().get(), 3);
    assert_eq!(restored.name(), "Project renamed after lowering");
}

#[test]
fn approve_and_execute_lower_milestone_classification_survives_a_later_h1_update_and_restart() {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic lowering rationale").unwrap();
    let prepared =
        milestone_prepared_intent("synthetic-delivery-h2a-prepared-milestone-3", &rationale);
    writer
        .prepare_lower_milestone_classification(
            PrepareLowerMilestoneClassification {
                id: milestone_id(),
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Confidential,
                rationale,
                context: context("prepare-milestone-3"),
            },
            prepared.clone(),
        )
        .unwrap();
    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-delivery-h2a-execute-idempotency-milestone-3").unwrap();
    let approval = approval_for(&prepared, &execute_idempotency_id);
    writer
        .approve_and_execute_lower_milestone_classification(
            ApproveAndExecuteLowerMilestoneClassification {
                approval,
                context: OperationContext {
                    idempotency_id: execute_idempotency_id,
                    correlation_id: CorrelationId::parse(
                        "synthetic-delivery-h2a-execute-correlation-milestone-3",
                    )
                    .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-delivery-h2a-audit-execute-milestone-3").unwrap(),
            ApprovalReceiptId::parse("synthetic-delivery-h2a-receipt-milestone-3").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();

    writer
        .update_milestone(
            UpdateMilestone {
                id: milestone_id(),
                expected_version: AggregateVersion::new(2).unwrap(),
                name: name("Milestone renamed after lowering"),
                verification_criteria: VerificationCriteria::parse(
                    "Evidence",
                    &context("parse").correlation_id,
                )
                .unwrap(),
                due_at: UtcTimestamp::from_unix_millis(3),
                classification: None,
                provenance: Provenance::UserEntered,
                context: context("post-lowering-update-milestone"),
            },
            AuditEventId::parse("synthetic-delivery-h2a-post-lowering-audit-milestone").unwrap(),
            UtcTimestamp::from_unix_millis(300),
        )
        .unwrap();

    drop(writer);
    let reopened = SqliteProductLedger::open(&_ledger.0).unwrap();
    let snapshot = reopened.load_delivery_persistence_snapshot().expect(
        "restart decode must succeed even after a lowering is followed by an ordinary update",
    );
    let restored = snapshot
        .milestones()
        .iter()
        .find(|value| value.id() == &milestone_id())
        .unwrap();
    assert_eq!(restored.classification(), DataClassification::Confidential);
    assert_eq!(restored.version().get(), 3);
    assert_eq!(restored.name(), "Milestone renamed after lowering");
}

#[test]
fn approve_and_execute_lower_initiative_classification_replay_returns_the_original_outcome_after_a_later_update(
) {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic lowering rationale").unwrap();
    let prepared =
        initiative_prepared_intent("synthetic-delivery-h2a-prepared-initiative-5", &rationale);
    writer
        .prepare_lower_initiative_classification(
            PrepareLowerInitiativeClassification {
                id: initiative_id(),
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Internal,
                rationale,
                context: context("prepare-initiative-5"),
            },
            prepared.clone(),
        )
        .unwrap();
    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-delivery-h2a-execute-idempotency-initiative-5").unwrap();
    let approval = approval_for(&prepared, &execute_idempotency_id);
    let execute_command = ApproveAndExecuteLowerInitiativeClassification {
        approval,
        context: OperationContext {
            idempotency_id: execute_idempotency_id,
            correlation_id: CorrelationId::parse(
                "synthetic-delivery-h2a-execute-correlation-initiative-5",
            )
            .unwrap(),
        },
    };
    let first = writer
        .approve_and_execute_lower_initiative_classification(
            execute_command.clone(),
            AuditEventId::parse("synthetic-delivery-h2a-audit-execute-initiative-5").unwrap(),
            ApprovalReceiptId::parse("synthetic-delivery-h2a-receipt-initiative-5").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    assert_eq!(first.record.version().get(), 2);
    assert_eq!(first.record.classification(), DataClassification::Internal);

    writer
        .update_initiative(
            UpdateInitiative {
                id: initiative_id(),
                expected_version: AggregateVersion::new(2).unwrap(),
                name: name("Initiative renamed after lowering"),
                defined_outcome: DefinedOutcome::parse("Outcome", &context("parse").correlation_id)
                    .unwrap(),
                classification: None,
                provenance: Provenance::UserEntered,
                context: context("post-lowering-update-initiative-5"),
            },
            AuditEventId::parse("synthetic-delivery-h2a-post-lowering-audit-initiative-5").unwrap(),
            UtcTimestamp::from_unix_millis(300),
        )
        .unwrap();

    // Replaying the SAME execute idempotency id must still return the
    // ORIGINAL v2 lowering outcome, not the record's current v3 state.
    let replay = writer
        .approve_and_execute_lower_initiative_classification(
            execute_command,
            AuditEventId::parse("synthetic-delivery-h2a-audit-execute-initiative-5").unwrap(),
            ApprovalReceiptId::parse("synthetic-delivery-h2a-receipt-initiative-5").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    assert_eq!(replay.record.version().get(), 2);
    assert_eq!(replay.record.classification(), DataClassification::Internal);
    assert_eq!(replay.record.name(), "Initiative");
}

#[test]
fn approve_and_execute_lower_initiative_classification_rejects_a_replay_with_a_mismatched_approval_idempotency_id(
) {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic lowering rationale").unwrap();
    let prepared =
        initiative_prepared_intent("synthetic-delivery-h2a-prepared-initiative-6", &rationale);
    writer
        .prepare_lower_initiative_classification(
            PrepareLowerInitiativeClassification {
                id: initiative_id(),
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Internal,
                rationale,
                context: context("prepare-initiative-6"),
            },
            prepared.clone(),
        )
        .unwrap();
    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-delivery-h2a-execute-idempotency-initiative-6").unwrap();
    let approval = approval_for(&prepared, &execute_idempotency_id);
    writer
        .approve_and_execute_lower_initiative_classification(
            ApproveAndExecuteLowerInitiativeClassification {
                approval,
                context: OperationContext {
                    idempotency_id: execute_idempotency_id.clone(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-delivery-h2a-execute-correlation-initiative-6",
                    )
                    .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-delivery-h2a-audit-execute-initiative-6").unwrap(),
            ApprovalReceiptId::parse("synthetic-delivery-h2a-receipt-initiative-6").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();

    // Same prepared_id/actor/digest as the committed approval, but a
    // different idempotency_id than the one it was actually approved
    // under -- submitted alongside a context that still claims the
    // original (already-committed) idempotency_id, to hit the replay
    // branch's stored-row lookup.
    let mismatched_approval = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("synthetic-delivery-h2a-execute-idempotency-initiative-6-different")
            .unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    let result = writer.approve_and_execute_lower_initiative_classification(
        ApproveAndExecuteLowerInitiativeClassification {
            approval: mismatched_approval,
            context: OperationContext {
                idempotency_id: execute_idempotency_id,
                correlation_id: CorrelationId::parse(
                    "synthetic-delivery-h2a-execute-correlation-initiative-6-replay",
                )
                .unwrap(),
            },
        },
        AuditEventId::parse("synthetic-delivery-h2a-audit-execute-initiative-6-replay").unwrap(),
        ApprovalReceiptId::parse("synthetic-delivery-h2a-receipt-initiative-6-replay").unwrap(),
        UtcTimestamp::from_unix_millis(200),
    );
    assert!(result.is_err());
}
