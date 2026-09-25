use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use pmc_domain::{
    audit::{
        AuditAction, AuditActor, AuditApprovalOutcome, AuditDisposition, AuditEffectCode,
        AuditEffectScope, AuditEvent, AuditEventCode, AuditEventIdSource, AuditExecutionOutcome,
        AuditModule, AuditPolicyOutcome, AuditTarget,
    },
    classification::DataClassification,
    delivery::{
        CommandIdentity, CreateInitiative, CreateMilestone, CreateProject, DefinedOutcome,
        DeliveryPersistenceResult, DeliveryPersistenceSnapshot, DeliveryReplayCapsule,
        InMemoryDeliveryService, Initiative, InitiativePersistenceRecord, Milestone,
        MilestonePersistenceRecord, OperationContext, Project, ProjectPersistenceRecord,
        RecordName, UpdateInitiative, UpdateMilestone, UpdateProject, VerificationCriteria,
    },
    identity::{
        AggregateVersion, AuditEventId, CorrelationId, IdempotencyId, InitiativeId, MilestoneId,
        ProjectId,
    },
    provenance::{Provenance, ProvenanceReference},
    time::{Clock, UtcTimestamp},
    DomainValueError,
};
use pmc_ledger::sqlite::SqliteProductLedger;
use rusqlite::{named_params, Connection, Row};

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

struct SyntheticLedger(PathBuf);

impl SyntheticLedger {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "pmc-synthetic-delivery-rehydrate-{}.sqlite3",
            NEXT_PATH.fetch_add(1, Ordering::Relaxed)
        )))
    }
}

impl Drop for SyntheticLedger {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", self.0.display(), suffix));
        }
    }
}

#[derive(Clone, Copy)]
struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(100)
    }
}

#[derive(Clone, Default)]
struct AuditIds(u64);
impl AuditEventIdSource for AuditIds {
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        let id = AuditEventId::parse(format!("sqlite-delivery-audit-{}", self.0))?;
        self.0 += 1;
        Ok(id)
    }
}

fn context(value: &str) -> OperationContext {
    OperationContext {
        correlation_id: CorrelationId::parse(format!("correlation-{value}")).unwrap(),
        idempotency_id: IdempotencyId::parse(format!("idempotency-{value}")).unwrap(),
    }
}

fn name(value: &str) -> RecordName {
    RecordName::parse(value, &context("parse").correlation_id).unwrap()
}

fn provenance() -> Provenance {
    Provenance::SyntheticFixture(ProvenanceReference::parse("sqlite-delivery").unwrap())
}

fn classification(value: &str) -> DataClassification {
    DataClassification::from_persisted(value).unwrap()
}

fn decode_provenance(kind: &str, reference: Option<String>) -> Provenance {
    match kind {
        "user_entered" => Provenance::UserEntered,
        "authoritative_transition" => Provenance::AuthoritativeTransition(
            ProvenanceReference::parse(reference.unwrap()).unwrap(),
        ),
        "synthetic_fixture" => {
            Provenance::SyntheticFixture(ProvenanceReference::parse(reference.unwrap()).unwrap())
        }
        _ => panic!("closed provenance kind"),
    }
}

struct Encoded<'a> {
    kind: &'static str,
    target: &'a str,
    expected: Option<i64>,
    name: &'a str,
    defined: Option<&'a str>,
    project: Option<&'a str>,
    criteria: Option<&'a str>,
    start: Option<i64>,
    end: Option<i64>,
    due: Option<i64>,
    class: Option<&'static str>,
    provenance: &'a Provenance,
}

fn encode_command(command: &CommandIdentity) -> Encoded<'_> {
    match command {
        CommandIdentity::CreateInitiative {
            id,
            name,
            defined_outcome,
            classification,
            provenance,
        } => Encoded {
            kind: "create_initiative",
            target: id.as_str(),
            expected: None,
            name: name.as_str(),
            defined: Some(defined_outcome.as_str()),
            project: None,
            criteria: None,
            start: None,
            end: None,
            due: None,
            class: classification.map(DataClassification::as_persisted),
            provenance,
        },
        CommandIdentity::UpdateInitiative {
            id,
            expected_version,
            name,
            defined_outcome,
            classification,
            provenance,
        } => Encoded {
            kind: "update_initiative",
            target: id.as_str(),
            expected: Some(expected_version.get() as i64),
            name: name.as_str(),
            defined: Some(defined_outcome.as_str()),
            project: None,
            criteria: None,
            start: None,
            end: None,
            due: None,
            class: classification.map(DataClassification::as_persisted),
            provenance,
        },
        CommandIdentity::CreateProject {
            id,
            name,
            start_at,
            end_at,
            classification,
            provenance,
        } => Encoded {
            kind: "create_project",
            target: id.as_str(),
            expected: None,
            name: name.as_str(),
            defined: None,
            project: None,
            criteria: None,
            start: Some(start_at.unix_millis()),
            end: Some(end_at.unix_millis()),
            due: None,
            class: classification.map(DataClassification::as_persisted),
            provenance,
        },
        CommandIdentity::UpdateProject {
            id,
            expected_version,
            name,
            start_at,
            end_at,
            classification,
            provenance,
        } => Encoded {
            kind: "update_project",
            target: id.as_str(),
            expected: Some(expected_version.get() as i64),
            name: name.as_str(),
            defined: None,
            project: None,
            criteria: None,
            start: Some(start_at.unix_millis()),
            end: Some(end_at.unix_millis()),
            due: None,
            class: classification.map(DataClassification::as_persisted),
            provenance,
        },
        CommandIdentity::CreateMilestone {
            id,
            project_id,
            name,
            verification_criteria,
            due_at,
            classification,
            provenance,
        } => Encoded {
            kind: "create_milestone",
            target: id.as_str(),
            expected: None,
            name: name.as_str(),
            defined: None,
            project: Some(project_id.as_str()),
            criteria: Some(verification_criteria.as_str()),
            start: None,
            end: None,
            due: Some(due_at.unix_millis()),
            class: classification.map(DataClassification::as_persisted),
            provenance,
        },
        CommandIdentity::UpdateMilestone {
            id,
            expected_version,
            name,
            verification_criteria,
            due_at,
            classification,
            provenance,
        } => Encoded {
            kind: "update_milestone",
            target: id.as_str(),
            expected: Some(expected_version.get() as i64),
            name: name.as_str(),
            defined: None,
            project: None,
            criteria: Some(verification_criteria.as_str()),
            start: None,
            end: None,
            due: Some(due_at.unix_millis()),
            class: classification.map(DataClassification::as_persisted),
            provenance,
        },
        // H2a lowering: `pmc-ledger`'s production delivery_repository.rs now
        // does call all six Prepare/ApproveAndExecute Lower<X>Classification
        // methods (see delivery_repository.rs and
        // sqlite-delivery-h2a-lower-classification.rs), but this file's own
        // fixture -- `all_six_delivery_commands_rehydrate_from_normalized_
        // historical_rows` -- only ever exercises the six H1 commands, and
        // these CommandIdentity variants carry no `Provenance` field for
        // `Encoded` to represent anyway. Left as a loud failure (not a
        // silent `_ => ..`) so a future fixture change that starts
        // producing one here is forced to extend this encoder rather than
        // silently falling through, matching the same precedent in
        // sqlite-portfolio-rehydration.rs.
        CommandIdentity::PrepareLowerInitiativeClassification { .. }
        | CommandIdentity::ApproveAndExecuteLowerInitiativeClassification { .. }
        | CommandIdentity::PrepareLowerProjectClassification { .. }
        | CommandIdentity::ApproveAndExecuteLowerProjectClassification { .. }
        | CommandIdentity::PrepareLowerMilestoneClassification { .. }
        | CommandIdentity::ApproveAndExecuteLowerMilestoneClassification { .. } => {
            unreachable!("this fixture only ever encodes the six H1 delivery commands")
        }
    }
}

struct ResultEncoded<'a> {
    kind: &'static str,
    id: &'a str,
    project: Option<&'a str>,
    name: &'a str,
    defined: Option<&'a str>,
    criteria: Option<&'a str>,
    start: Option<i64>,
    end: Option<i64>,
    due: Option<i64>,
    class: &'static str,
    provenance: &'a Provenance,
    version: i64,
    created: i64,
    updated: i64,
}

fn encode_result(result: &DeliveryPersistenceResult) -> ResultEncoded<'_> {
    match result {
        DeliveryPersistenceResult::Initiative(v) => ResultEncoded {
            kind: "initiative",
            id: v.id().as_str(),
            project: None,
            name: v.name(),
            defined: Some(v.defined_outcome()),
            criteria: None,
            start: None,
            end: None,
            due: None,
            class: v.classification().as_persisted(),
            provenance: v.provenance(),
            version: v.version().get() as i64,
            created: v.created_at().unix_millis(),
            updated: v.updated_at().unix_millis(),
        },
        DeliveryPersistenceResult::Project(v) => ResultEncoded {
            kind: "project",
            id: v.id().as_str(),
            project: None,
            name: v.name(),
            defined: None,
            criteria: None,
            start: Some(v.start_at().unix_millis()),
            end: Some(v.end_at().unix_millis()),
            due: None,
            class: v.classification().as_persisted(),
            provenance: v.provenance(),
            version: v.version().get() as i64,
            created: v.created_at().unix_millis(),
            updated: v.updated_at().unix_millis(),
        },
        DeliveryPersistenceResult::Milestone(v) => ResultEncoded {
            kind: "milestone",
            id: v.id().as_str(),
            project: Some(v.project_id().as_str()),
            name: v.name(),
            defined: None,
            criteria: Some(v.verification_criteria()),
            start: None,
            end: None,
            due: Some(v.due_at().unix_millis()),
            class: v.classification().as_persisted(),
            provenance: v.provenance(),
            version: v.version().get() as i64,
            created: v.created_at().unix_millis(),
            updated: v.updated_at().unix_millis(),
        },
    }
}

fn target_parts(target: &AuditTarget) -> (&'static str, &str) {
    match target {
        AuditTarget::Initiative(id) => ("initiative", id.as_str()),
        AuditTarget::Project(id) => ("project", id.as_str()),
        AuditTarget::Milestone(id) => ("milestone", id.as_str()),
        _ => panic!("Delivery fixture emitted an unexpected audit target"),
    }
}

fn decode_target(kind: &str, id: String) -> AuditTarget {
    match kind {
        "initiative" => AuditTarget::Initiative(InitiativeId::parse(id).unwrap()),
        "project" => AuditTarget::Project(ProjectId::parse(id).unwrap()),
        "milestone" => AuditTarget::Milestone(MilestoneId::parse(id).unwrap()),
        _ => panic!("closed Delivery audit target"),
    }
}

fn insert_audit(connection: &Connection, event: &AuditEvent) {
    let (target_kind, target_id) = target_parts(event.target());
    connection
        .execute(
            "INSERT INTO audit_events VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            (
                event.id().as_str(),
                event.occurred_at().unix_millis(),
                event.actor().as_persisted(),
                event.module().as_persisted(),
                event.code().as_str(),
                target_kind,
                target_id,
                event.correlation_id().as_str(),
                event.policy_outcome().as_persisted(),
                event.approval_outcome().as_persisted(),
                event.execution_outcome().as_persisted(),
                event.effect_scope().as_persisted(),
            ),
        )
        .unwrap();
    for (ordinal, effect) in event.actual_effects().iter().enumerate() {
        connection
            .execute(
                "INSERT INTO audit_effects VALUES(?1,?2,?3,?4,?5,?6)",
                (
                    event.id().as_str(),
                    ordinal as i64,
                    effect.as_str(),
                    event.effect_scope().as_persisted(),
                    target_kind,
                    target_id,
                ),
            )
            .unwrap();
    }
}

fn insert_capsule(connection: &Connection, capsule: &DeliveryReplayCapsule, audits: &[AuditEvent]) {
    let c = encode_command(capsule.command());
    let r = encode_result(capsule.result());
    connection.execute("INSERT INTO idempotency_outcomes VALUES('delivery',?1,?2,'synthetic-digest','succeeded',?3,100)", (c.kind,capsule.idempotency_id().as_str(),r.id)).unwrap();
    connection
        .execute(
            "INSERT INTO delivery_idempotency_outcomes VALUES('delivery',?1,?2,?1,?3,?4)",
            (
                c.kind,
                capsule.idempotency_id().as_str(),
                capsule.correlation_id().as_str(),
                capsule.operation_ordinal() as i64,
            ),
        )
        .unwrap();
    connection.execute(
        "INSERT INTO delivery_command_results VALUES('delivery',:operation,:idempotency,:kind,:target,:expected,:name,:defined,:project,:criteria,:start,:end,:due,:class,:pkind,:pref,:rkind,:rid,:rproject,:rname,:rdefined,:rcriteria,:rstart,:rend,:rdue,:rclass,:rpkind,:rpref,:version,:created,:updated)",
        named_params! { ":operation":c.kind,":idempotency":capsule.idempotency_id().as_str(),":kind":c.kind,":target":c.target,":expected":c.expected,":name":c.name,":defined":c.defined,":project":c.project,":criteria":c.criteria,":start":c.start,":end":c.end,":due":c.due,":class":c.class,":pkind":c.provenance.kind_persisted(),":pref":c.provenance.reference().map(|v|v.as_str()),":rkind":r.kind,":rid":r.id,":rproject":r.project,":rname":r.name,":rdefined":r.defined,":rcriteria":r.criteria,":rstart":r.start,":rend":r.end,":rdue":r.due,":rclass":r.class,":rpkind":r.provenance.kind_persisted(),":rpref":r.provenance.reference().map(|v|v.as_str()),":version":r.version,":created":r.created,":updated":r.updated }
    ).unwrap();
    for (ordinal, audit) in capsule.audit_event_ids().iter().enumerate() {
        let event = audits
            .iter()
            .find(|event| event.id() == audit)
            .expect("capsule audit must exist");
        insert_audit(connection, event);
        connection
            .execute(
                "INSERT INTO delivery_idempotency_outcome_audits VALUES('delivery',?1,?2,?3,?4)",
                (
                    c.kind,
                    capsule.idempotency_id().as_str(),
                    ordinal as i64,
                    audit.as_str(),
                ),
            )
            .unwrap();
    }
    for (ordinal, mutation) in capsule.derived_milestone_mutations().iter().enumerate() {
        connection.execute("INSERT INTO delivery_derived_milestone_mutations VALUES('delivery',?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)", (c.kind,capsule.idempotency_id().as_str(),ordinal as i64,mutation.milestone_id().as_str(),mutation.previous_version().get() as i64,mutation.resulting_version().get() as i64,mutation.previous_classification().as_persisted(),mutation.resulting_classification().as_persisted(),mutation.previous_updated_at().unix_millis(),mutation.resulting_updated_at().unix_millis(),mutation.audit_event_id().as_str())).unwrap();
    }
}

struct Raw {
    operation: String,
    idempotency: String,
    correlation: String,
    ordinal: i64,
    kind: String,
    target: String,
    expected: Option<i64>,
    name: String,
    defined: Option<String>,
    project: Option<String>,
    criteria: Option<String>,
    start: Option<i64>,
    end: Option<i64>,
    due: Option<i64>,
    class: Option<String>,
    pkind: String,
    pref: Option<String>,
    rkind: String,
    rid: String,
    rproject: Option<String>,
    rname: String,
    rdefined: Option<String>,
    rcriteria: Option<String>,
    rstart: Option<i64>,
    rend: Option<i64>,
    rdue: Option<i64>,
    rclass: String,
    rpkind: String,
    rpref: Option<String>,
    version: i64,
    created: i64,
    updated: i64,
}

fn raw(row: &Row<'_>) -> rusqlite::Result<Raw> {
    Ok(Raw {
        operation: row.get(0)?,
        idempotency: row.get(1)?,
        correlation: row.get(2)?,
        ordinal: row.get(3)?,
        kind: row.get(4)?,
        target: row.get(5)?,
        expected: row.get(6)?,
        name: row.get(7)?,
        defined: row.get(8)?,
        project: row.get(9)?,
        criteria: row.get(10)?,
        start: row.get(11)?,
        end: row.get(12)?,
        due: row.get(13)?,
        class: row.get(14)?,
        pkind: row.get(15)?,
        pref: row.get(16)?,
        rkind: row.get(17)?,
        rid: row.get(18)?,
        rproject: row.get(19)?,
        rname: row.get(20)?,
        rdefined: row.get(21)?,
        rcriteria: row.get(22)?,
        rstart: row.get(23)?,
        rend: row.get(24)?,
        rdue: row.get(25)?,
        rclass: row.get(26)?,
        rpkind: row.get(27)?,
        rpref: row.get(28)?,
        version: row.get(29)?,
        created: row.get(30)?,
        updated: row.get(31)?,
    })
}

fn decode(row: Raw, connection: &Connection) -> DeliveryReplayCapsule {
    let correlation = CorrelationId::parse(row.correlation).unwrap();
    let p = decode_provenance(&row.pkind, row.pref);
    let class = row.class.as_deref().map(classification);
    let expected = || AggregateVersion::new(row.expected.unwrap() as u64).unwrap();
    let command = match row.kind.as_str() {
        "create_initiative" => CommandIdentity::CreateInitiative {
            id: InitiativeId::parse(&row.target).unwrap(),
            name: RecordName::parse(&row.name, &correlation).unwrap(),
            defined_outcome: DefinedOutcome::parse(row.defined.unwrap(), &correlation).unwrap(),
            classification: class,
            provenance: p,
        },
        "update_initiative" => CommandIdentity::UpdateInitiative {
            id: InitiativeId::parse(&row.target).unwrap(),
            expected_version: expected(),
            name: RecordName::parse(&row.name, &correlation).unwrap(),
            defined_outcome: DefinedOutcome::parse(row.defined.unwrap(), &correlation).unwrap(),
            classification: class,
            provenance: p,
        },
        "create_project" => CommandIdentity::CreateProject {
            id: ProjectId::parse(&row.target).unwrap(),
            name: RecordName::parse(&row.name, &correlation).unwrap(),
            start_at: UtcTimestamp::from_unix_millis(row.start.unwrap()),
            end_at: UtcTimestamp::from_unix_millis(row.end.unwrap()),
            classification: class,
            provenance: p,
        },
        "update_project" => CommandIdentity::UpdateProject {
            id: ProjectId::parse(&row.target).unwrap(),
            expected_version: expected(),
            name: RecordName::parse(&row.name, &correlation).unwrap(),
            start_at: UtcTimestamp::from_unix_millis(row.start.unwrap()),
            end_at: UtcTimestamp::from_unix_millis(row.end.unwrap()),
            classification: class,
            provenance: p,
        },
        "create_milestone" => CommandIdentity::CreateMilestone {
            id: MilestoneId::parse(&row.target).unwrap(),
            project_id: ProjectId::parse(row.project.unwrap()).unwrap(),
            name: RecordName::parse(&row.name, &correlation).unwrap(),
            verification_criteria: VerificationCriteria::parse(row.criteria.unwrap(), &correlation)
                .unwrap(),
            due_at: UtcTimestamp::from_unix_millis(row.due.unwrap()),
            classification: class,
            provenance: p,
        },
        "update_milestone" => CommandIdentity::UpdateMilestone {
            id: MilestoneId::parse(&row.target).unwrap(),
            expected_version: expected(),
            name: RecordName::parse(&row.name, &correlation).unwrap(),
            verification_criteria: VerificationCriteria::parse(row.criteria.unwrap(), &correlation)
                .unwrap(),
            due_at: UtcTimestamp::from_unix_millis(row.due.unwrap()),
            classification: class,
            provenance: p,
        },
        _ => panic!("closed command kind"),
    };
    let rp = decode_provenance(&row.rpkind, row.rpref);
    let version = AggregateVersion::new(row.version as u64).unwrap();
    let created = UtcTimestamp::from_unix_millis(row.created);
    let updated = UtcTimestamp::from_unix_millis(row.updated);
    let result = match row.rkind.as_str() {
        "initiative" => DeliveryPersistenceResult::Initiative(
            Initiative::rehydrate(InitiativePersistenceRecord {
                id: InitiativeId::parse(row.rid).unwrap(),
                name: RecordName::parse(row.rname, &correlation).unwrap(),
                defined_outcome: DefinedOutcome::parse(row.rdefined.unwrap(), &correlation)
                    .unwrap(),
                classification: classification(&row.rclass),
                provenance: rp,
                version,
                created_at: created,
                updated_at: updated,
            })
            .unwrap(),
        ),
        "project" => DeliveryPersistenceResult::Project(
            Project::rehydrate(ProjectPersistenceRecord {
                id: ProjectId::parse(row.rid).unwrap(),
                name: RecordName::parse(row.rname, &correlation).unwrap(),
                start_at: UtcTimestamp::from_unix_millis(row.rstart.unwrap()),
                end_at: UtcTimestamp::from_unix_millis(row.rend.unwrap()),
                classification: classification(&row.rclass),
                provenance: rp,
                version,
                created_at: created,
                updated_at: updated,
            })
            .unwrap(),
        ),
        "milestone" => DeliveryPersistenceResult::Milestone(
            Milestone::rehydrate(MilestonePersistenceRecord {
                id: MilestoneId::parse(row.rid).unwrap(),
                project_id: ProjectId::parse(row.rproject.unwrap()).unwrap(),
                name: RecordName::parse(row.rname, &correlation).unwrap(),
                verification_criteria: VerificationCriteria::parse(
                    row.rcriteria.unwrap(),
                    &correlation,
                )
                .unwrap(),
                due_at: UtcTimestamp::from_unix_millis(row.rdue.unwrap()),
                classification: classification(&row.rclass),
                provenance: rp,
                version,
                created_at: created,
                updated_at: updated,
            })
            .unwrap(),
        ),
        _ => panic!("closed result kind"),
    };
    let audit_event_ids=connection.prepare("SELECT audit_event_id FROM delivery_idempotency_outcome_audits WHERE namespace='delivery' AND operation=?1 AND idempotency_id=?2 ORDER BY ordinal").unwrap().query_map((&row.operation,&row.idempotency),|r|r.get::<_,String>(0)).unwrap().map(|v|AuditEventId::parse(v.unwrap()).unwrap()).collect();
    let derived_milestone_mutations=connection.prepare("SELECT milestone_id,previous_version,resulting_version,previous_classification,resulting_classification,previous_updated_at,resulting_updated_at,audit_event_id FROM delivery_derived_milestone_mutations WHERE namespace='delivery' AND operation=?1 AND idempotency_id=?2 ORDER BY ordinal").unwrap().query_map((&row.operation,&row.idempotency),|r|Ok(pmc_domain::delivery::DerivedMilestoneMutation::new(MilestoneId::parse(r.get::<_,String>(0)?).unwrap(),AggregateVersion::new(r.get::<_,i64>(1)? as u64).unwrap(),AggregateVersion::new(r.get::<_,i64>(2)? as u64).unwrap(),classification(&r.get::<_,String>(3)?),classification(&r.get::<_,String>(4)?),UtcTimestamp::from_unix_millis(r.get(5)?),UtcTimestamp::from_unix_millis(r.get(6)?),AuditEventId::parse(r.get::<_,String>(7)?).unwrap()))).unwrap().map(Result::unwrap).collect();
    DeliveryReplayCapsule::new(
        IdempotencyId::parse(row.idempotency).unwrap(),
        command,
        result,
        correlation,
        audit_event_ids,
        row.ordinal as u64,
        derived_milestone_mutations,
    )
}

fn decode_audits(connection: &Connection) -> Vec<AuditEvent> {
    let mut statement=connection.prepare("SELECT e.id,e.occurred_at,e.actor,e.module,e.event_code,e.target_type,e.target_id,e.correlation_id,e.policy_outcome,e.approval_outcome,e.execution_outcome,e.effect_scope FROM delivery_idempotency_outcomes o JOIN delivery_idempotency_outcome_audits a USING(namespace,operation,idempotency_id) JOIN audit_events e ON e.id=a.audit_event_id ORDER BY o.operation_ordinal,a.ordinal").unwrap();
    statement.query_map([],|row| {
        let id:String=row.get(0)?;
        let target_kind:String=row.get(5)?;
        let target_id:String=row.get(6)?;
        let effect_scope:String=row.get(11)?;
        let effects=connection.prepare("SELECT effect_code,scope,target_type,target_id FROM audit_effects WHERE audit_event_id=?1 ORDER BY ordinal").unwrap().query_map([&id],|effect|Ok((effect.get::<_,String>(0)?,effect.get::<_,String>(1)?,effect.get::<_,Option<String>>(2)?,effect.get::<_,Option<String>>(3)?))).unwrap().map(|effect|{
            let (code,scope,effect_target_kind,effect_target_id)=effect.unwrap();
            assert_eq!(scope,effect_scope);
            assert_eq!(effect_target_kind.as_deref(),Some(target_kind.as_str()));
            assert_eq!(effect_target_id.as_deref(),Some(target_id.as_str()));
            AuditEffectCode::parse(code).unwrap()
        }).collect();
        let action=AuditAction::new(
            AuditModule::from_persisted(&row.get::<_,String>(3)?).unwrap(),
            AuditEventCode::parse(row.get::<_,String>(4)?).unwrap(),
            decode_target(&target_kind,target_id),
        );
        let disposition=AuditDisposition::new(
            AuditPolicyOutcome::from_persisted(&row.get::<_,String>(8)?).unwrap(),
            AuditApprovalOutcome::from_persisted(&row.get::<_,String>(9)?).unwrap(),
            AuditExecutionOutcome::from_persisted(&row.get::<_,String>(10)?).unwrap(),
            AuditEffectScope::from_persisted(&effect_scope).unwrap(),
            effects,
        ).unwrap();
        Ok(AuditEvent::new(
            AuditEventId::parse(id).unwrap(),
            UtcTimestamp::from_unix_millis(row.get(1)?),
            AuditActor::from_persisted(&row.get::<_,String>(2)?).unwrap(),
            action,
            CorrelationId::parse(row.get::<_,String>(7)?).unwrap(),
            disposition,
        ))
    }).unwrap().map(Result::unwrap).collect()
}

fn assert_capsules(actual: &[DeliveryReplayCapsule], expected: &[DeliveryReplayCapsule]) {
    assert_eq!(actual.len(), expected.len());
    let mut actual = actual.iter().collect::<Vec<_>>();
    let mut expected = expected.iter().collect::<Vec<_>>();
    actual.sort_by_key(|value| value.operation_ordinal());
    expected.sort_by_key(|value| value.operation_ordinal());
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert_eq!(actual.idempotency_id(), expected.idempotency_id());
        assert_eq!(actual.command(), expected.command());
        assert_eq!(actual.result(), expected.result());
        assert_eq!(actual.correlation_id(), expected.correlation_id());
        assert_eq!(actual.audit_event_ids(), expected.audit_event_ids());
        assert_eq!(actual.operation_ordinal(), expected.operation_ordinal());
        assert_eq!(
            actual.derived_milestone_mutations(),
            expected.derived_milestone_mutations()
        );
    }
}

fn replay(
    service: &mut InMemoryDeliveryService<FixedClock, AuditIds>,
    capsule: &DeliveryReplayCapsule,
) {
    let operation_context = OperationContext {
        correlation_id: CorrelationId::parse("different-replay-correlation").unwrap(),
        idempotency_id: capsule.idempotency_id().clone(),
    };
    let actual = match capsule.command() {
        CommandIdentity::CreateInitiative {
            id,
            name,
            defined_outcome,
            classification,
            provenance,
        } => DeliveryPersistenceResult::Initiative(
            service
                .create_initiative(CreateInitiative {
                    context: operation_context,
                    id: id.clone(),
                    name: name.clone(),
                    defined_outcome: defined_outcome.clone(),
                    classification: *classification,
                    provenance: provenance.clone(),
                })
                .unwrap(),
        ),
        CommandIdentity::UpdateInitiative {
            id,
            expected_version,
            name,
            defined_outcome,
            classification,
            provenance,
        } => DeliveryPersistenceResult::Initiative(
            service
                .update_initiative(UpdateInitiative {
                    context: operation_context,
                    id: id.clone(),
                    expected_version: *expected_version,
                    name: name.clone(),
                    defined_outcome: defined_outcome.clone(),
                    classification: *classification,
                    provenance: provenance.clone(),
                })
                .unwrap(),
        ),
        CommandIdentity::CreateProject {
            id,
            name,
            start_at,
            end_at,
            classification,
            provenance,
        } => DeliveryPersistenceResult::Project(
            service
                .create_project(CreateProject {
                    context: operation_context,
                    id: id.clone(),
                    name: name.clone(),
                    start_at: *start_at,
                    end_at: *end_at,
                    classification: *classification,
                    provenance: provenance.clone(),
                })
                .unwrap(),
        ),
        CommandIdentity::UpdateProject {
            id,
            expected_version,
            name,
            start_at,
            end_at,
            classification,
            provenance,
        } => DeliveryPersistenceResult::Project(
            service
                .update_project(UpdateProject {
                    context: operation_context,
                    id: id.clone(),
                    expected_version: *expected_version,
                    name: name.clone(),
                    start_at: *start_at,
                    end_at: *end_at,
                    classification: *classification,
                    provenance: provenance.clone(),
                })
                .unwrap(),
        ),
        CommandIdentity::CreateMilestone {
            id,
            project_id,
            name,
            verification_criteria,
            due_at,
            classification,
            provenance,
        } => DeliveryPersistenceResult::Milestone(
            service
                .create_milestone(CreateMilestone {
                    context: operation_context,
                    id: id.clone(),
                    project_id: project_id.clone(),
                    name: name.clone(),
                    verification_criteria: verification_criteria.clone(),
                    due_at: *due_at,
                    classification: *classification,
                    provenance: provenance.clone(),
                })
                .unwrap(),
        ),
        CommandIdentity::UpdateMilestone {
            id,
            expected_version,
            name,
            verification_criteria,
            due_at,
            classification,
            provenance,
        } => DeliveryPersistenceResult::Milestone(
            service
                .update_milestone(UpdateMilestone {
                    context: operation_context,
                    id: id.clone(),
                    expected_version: *expected_version,
                    name: name.clone(),
                    verification_criteria: verification_criteria.clone(),
                    due_at: *due_at,
                    classification: *classification,
                    provenance: provenance.clone(),
                })
                .unwrap(),
        ),
        // See the matching catch-all in `encode_command`: this fixture only
        // ever replays the six H1 delivery commands, even though production
        // delivery_repository.rs now handles Lower<X>Classification too.
        CommandIdentity::PrepareLowerInitiativeClassification { .. }
        | CommandIdentity::ApproveAndExecuteLowerInitiativeClassification { .. }
        | CommandIdentity::PrepareLowerProjectClassification { .. }
        | CommandIdentity::ApproveAndExecuteLowerProjectClassification { .. }
        | CommandIdentity::PrepareLowerMilestoneClassification { .. }
        | CommandIdentity::ApproveAndExecuteLowerMilestoneClassification { .. } => {
            unreachable!("this fixture only ever replays the six H1 delivery commands")
        }
    };
    assert_eq!(&actual, capsule.result());
}

#[test]
fn all_six_delivery_commands_rehydrate_from_normalized_historical_rows() {
    let mut service = InMemoryDeliveryService::new(FixedClock, AuditIds::default());
    let initiative = service
        .create_initiative(CreateInitiative {
            context: context("ci"),
            id: InitiativeId::parse("initiative-sql").unwrap(),
            name: name("Initiative"),
            defined_outcome: DefinedOutcome::parse("Outcome", &context("parse").correlation_id)
                .unwrap(),
            classification: Some(DataClassification::Internal),
            provenance: provenance(),
        })
        .unwrap();
    service
        .update_initiative(UpdateInitiative {
            context: context("ui"),
            id: initiative.id().clone(),
            expected_version: initiative.version(),
            name: name("Initiative updated"),
            defined_outcome: DefinedOutcome::parse(
                "Outcome updated",
                &context("parse").correlation_id,
            )
            .unwrap(),
            classification: None,
            provenance: provenance(),
        })
        .unwrap();
    let project = service
        .create_project(CreateProject {
            context: context("cp"),
            id: ProjectId::parse("project-sql").unwrap(),
            name: name("Project"),
            start_at: UtcTimestamp::from_unix_millis(1),
            end_at: UtcTimestamp::from_unix_millis(2),
            classification: Some(DataClassification::Internal),
            provenance: provenance(),
        })
        .unwrap();
    let milestone = service
        .create_milestone(CreateMilestone {
            context: context("cm"),
            id: MilestoneId::parse("milestone-sql").unwrap(),
            project_id: project.id().clone(),
            name: name("Milestone"),
            verification_criteria: VerificationCriteria::parse(
                "Evidence",
                &context("parse").correlation_id,
            )
            .unwrap(),
            due_at: UtcTimestamp::from_unix_millis(3),
            classification: None,
            provenance: provenance(),
        })
        .unwrap();
    service
        .update_project(UpdateProject {
            context: context("up"),
            id: project.id().clone(),
            expected_version: project.version(),
            name: name("Project updated"),
            start_at: project.start_at(),
            end_at: project.end_at(),
            classification: Some(DataClassification::Restricted),
            provenance: provenance(),
        })
        .unwrap();
    let inherited = service
        .milestones()
        .into_iter()
        .find(|v| v.id() == milestone.id())
        .unwrap();
    service
        .update_milestone(UpdateMilestone {
            context: context("um"),
            id: inherited.id().clone(),
            expected_version: inherited.version(),
            name: name("Milestone updated"),
            verification_criteria: VerificationCriteria::parse(
                "Evidence updated",
                &context("parse").correlation_id,
            )
            .unwrap(),
            due_at: inherited.due_at(),
            classification: None,
            provenance: provenance(),
        })
        .unwrap();
    let snapshot = service.persistence_snapshot();
    let ledger = SyntheticLedger::new();
    drop(SqliteProductLedger::open(&ledger.0).unwrap());
    let connection = Connection::open(&ledger.0).unwrap();
    connection.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    for project in snapshot.projects() {
        connection
            .execute(
                "INSERT INTO aggregate_registry VALUES(?1,'project',?2,?3,?4,?5)",
                (
                    project.id().as_str(),
                    project.version().get() as i64,
                    project.classification().as_persisted(),
                    project.created_at().unix_millis(),
                    project.updated_at().unix_millis(),
                ),
            )
            .unwrap();
        connection.execute("INSERT INTO projects(id,name,start_at,end_at,provenance_kind,provenance_reference) VALUES(?1,?2,?3,?4,?5,?6)",(project.id().as_str(),project.name(),project.start_at().unix_millis(),project.end_at().unix_millis(),project.provenance().kind_persisted(),project.provenance().reference().map(|v|v.as_str()))).unwrap();
    }
    for milestone in snapshot.milestones() {
        connection
            .execute(
                "INSERT INTO aggregate_registry VALUES(?1,'milestone',?2,?3,?4,?5)",
                (
                    milestone.id().as_str(),
                    milestone.version().get() as i64,
                    milestone.classification().as_persisted(),
                    milestone.created_at().unix_millis(),
                    milestone.updated_at().unix_millis(),
                ),
            )
            .unwrap();
        connection.execute("INSERT INTO milestones(id,project_id,name,verification_criteria,due_at,provenance_kind,provenance_reference) VALUES(?1,?2,?3,?4,?5,?6,?7)",(milestone.id().as_str(),milestone.project_id().as_str(),milestone.name(),milestone.verification_criteria(),milestone.due_at().unix_millis(),milestone.provenance().kind_persisted(),milestone.provenance().reference().map(|v|v.as_str()))).unwrap();
    }
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    for capsule in snapshot.replay() {
        insert_capsule(&connection, capsule, snapshot.audits());
    }
    connection.execute_batch("COMMIT").unwrap();
    let query="SELECT r.operation,r.idempotency_id,o.correlation_id,o.operation_ordinal,r.command_kind,r.command_target_id,r.command_expected_version,r.command_name,r.command_defined_outcome,r.command_project_id,r.command_verification_criteria,r.command_start_at,r.command_end_at,r.command_due_at,r.command_classification,r.command_provenance_kind,r.command_provenance_reference,r.result_kind,r.result_id,r.result_project_id,r.result_name,r.result_defined_outcome,r.result_verification_criteria,r.result_start_at,r.result_end_at,r.result_due_at,r.result_classification,r.result_provenance_kind,r.result_provenance_reference,r.result_version,r.result_created_at,r.result_updated_at FROM delivery_command_results r JOIN delivery_idempotency_outcomes o USING(namespace,operation,idempotency_id) ORDER BY o.operation_ordinal";
    let reconstructed_replay = connection
        .prepare(query)
        .unwrap()
        .query_map([], raw)
        .unwrap()
        .map(|v| decode(v.unwrap(), &connection))
        .collect::<Vec<_>>();
    assert_capsules(&reconstructed_replay, snapshot.replay());
    let reconstructed_audits = decode_audits(&connection);
    assert_eq!(reconstructed_audits, snapshot.audits());
    let validated = DeliveryPersistenceSnapshot::validate(
        snapshot.initiatives().to_vec(),
        snapshot.projects().to_vec(),
        snapshot.milestones().to_vec(),
        reconstructed_replay,
        reconstructed_audits,
    )
    .unwrap();
    let mut restored =
        InMemoryDeliveryService::rehydrate(FixedClock, AuditIds::default(), validated);
    assert_capsules(restored.persistence_snapshot().replay(), snapshot.replay());
    assert_eq!(restored.audit_events(), service.audit_events());
    let audit_count = restored.audit_events().len();
    for capsule in snapshot.replay() {
        replay(&mut restored, capsule);
    }
    assert_eq!(restored.audit_events().len(), audit_count);
}
