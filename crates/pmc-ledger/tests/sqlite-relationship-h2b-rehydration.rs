#![allow(clippy::result_large_err)]

//! Synthetic, SQLite-only Relationship/H2b persistence contract.

use pmc_domain::{
    audit::{
        AuditAction, AuditActor, AuditApprovalOutcome, AuditDisposition, AuditEffectCode,
        AuditEffectScope, AuditEvent, AuditEventCode, AuditEventIdSource, AuditExecutionOutcome,
        AuditModule, AuditPolicyOutcome, AuditTarget,
    },
    classification::DataClassification,
    error::{DomainError, ErrorCode, MessageKey},
    execution::{
        ApproveAndExecuteRemoveRelationship, CancellationPolicy, ExecutionIdSource, PayloadDigest,
        PrepareRemoveRelationship, PreparedIntent, RecoveryEvidence, RecoveryEvidencePort,
        RemovalEffect, RemovalPolicyDecision,
    },
    identity::{
        AggregateVersion, AuditEventId, CorrelationId, IdempotencyId, InitiativeId, KpiId,
        MilestoneId, PortfolioId, PreparedIntentId, ProductId, ProjectId, RecoveryEvidenceId,
        RelationshipId, RoadmapId, StakeholderId,
    },
    relationships::{
        CreateStakeholder, EndpointSnapshot, InMemoryEndpointCatalog, InMemoryRelationshipService,
        LinkInitiativeProject, LinkPortfolioInitiative, LinkPortfolioProduct, LinkProductKpi,
        LinkProductRoadmap, LinkProjectProduct, LinkStakeholderRelationship, PortfolioSnapshot,
        ProductSnapshot, RelationshipH2bPersistenceCommand, RelationshipH2bPersistenceResult,
        RelationshipH2bPersistenceSnapshot, RelationshipKind, RelationshipPersistenceCommand,
        RelationshipPersistenceRecord, RelationshipPersistenceResult,
        RelationshipPersistenceSnapshot, RelationshipRecord, RelationshipReplayCapsule,
        StakeholderKind, StakeholderPersistenceRecord, StakeholderRecord,
        StakeholderRelationshipPurpose, StakeholderSubject, UpdateStakeholderDetails,
    },
    time::{Clock, UtcTimestamp},
    DomainValueError,
};
use pmc_ledger::sqlite::SqliteProductLedger;
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct TempLedger(PathBuf);
impl TempLedger {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "pmc-rel-h2b-{}.sqlite3",
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}

fn open_test_connection(path: &Path) -> Connection {
    let connection = Connection::open(path).unwrap();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "foreign_keys", |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    connection
}
impl Drop for TempLedger {
    fn drop(&mut self) {
        for s in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", self.0.display(), s));
        }
    }
}
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
        AuditEventId::parse(format!("sqlite-rel-audit-{}", self.0))
    }
}
#[derive(Clone)]
struct ExecIds(u64);
impl ExecutionIdSource for ExecIds {
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        self.0 += 1;
        PreparedIntentId::parse(format!("sqlite-rel-prepared-{}", self.0))
    }
    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<pmc_domain::identity::ApprovalReceiptId, DomainValueError> {
        self.0 += 1;
        pmc_domain::identity::ApprovalReceiptId::parse(format!("sqlite-rel-receipt-{}", self.0))
    }
}
#[derive(Clone, Copy)]
struct AllowRemoval;
impl pmc_domain::execution::RemovalPolicyPort for AllowRemoval {
    fn allow_relationship_removal(&self, _: &RelationshipId, _: DataClassification) -> bool {
        true
    }
}
#[derive(Clone, Copy)]
struct AllowApproval;
impl pmc_domain::execution::ApprovalAuthorizationPort for AllowApproval {
    fn authorize_relationship_removal(&self, a: AuditActor) -> bool {
        a == AuditActor::HeadOfProducts
    }
}
#[derive(Clone)]
struct Recovery(RecoveryEvidence);
impl RecoveryEvidencePort for Recovery {
    fn recovery_evidence(&self, _: &RelationshipId) -> Option<RecoveryEvidence> {
        Some(self.0.clone())
    }
}
type Service = InMemoryRelationshipService<
    FixedClock,
    InMemoryEndpointCatalog,
    AuditIds,
    ExecIds,
    AllowRemoval,
    AllowApproval,
>;
fn ctx(s: &str) -> pmc_domain::relationships::OperationContext {
    pmc_domain::relationships::OperationContext {
        idempotency_id: IdempotencyId::parse(format!("sqlite-rel-{s}")).unwrap(),
        correlation_id: CorrelationId::parse(format!("sqlite-rel-correlation-{s}")).unwrap(),
    }
}
fn linked_fixture() -> Service {
    let mut s = InMemoryRelationshipService::with_execution_authorities(
        FixedClock,
        InMemoryEndpointCatalog::new([
            EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
                PortfolioId::parse("portfolio-sqlite-h2b").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Public,
            )),
            EndpointSnapshot::Product(ProductSnapshot::new(
                ProductId::parse("product-sqlite-h2b").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Internal,
            )),
        ]),
        AuditIds::default(),
        ExecIds(100),
        AllowRemoval,
        AllowApproval,
    );
    let rid = RelationshipId::parse("relationship-sqlite-h2b").unwrap();
    s.link_portfolio_product(LinkPortfolioProduct {
        id: rid.clone(),
        portfolio_id: PortfolioId::parse("portfolio-sqlite-h2b").unwrap(),
        product_id: ProductId::parse("product-sqlite-h2b").unwrap(),
        expected_portfolio_version: AggregateVersion::initial(),
        expected_product_version: AggregateVersion::initial(),
        context: ctx("link"),
    })
    .unwrap();
    s
}
fn fixture() -> Service {
    let mut s = linked_fixture();
    let rid = RelationshipId::parse("relationship-sqlite-h2b").unwrap();
    let ev = Recovery(
        RecoveryEvidence::new(
            RecoveryEvidenceId::parse("recovery-sqlite-h2b").unwrap(),
            "synthetic recovery",
            UtcTimestamp::from_unix_millis(900),
            rid.clone(),
            true,
        )
        .unwrap(),
    );
    s.prepare_remove_relationship(
        PrepareRemoveRelationship {
            relationship_id: rid,
            context: ctx("prepare"),
        },
        &ev,
    )
    .unwrap();
    s
}

fn ordinary_full_fixture() -> Service {
    let portfolio = PortfolioId::parse("ordinary-portfolio").unwrap();
    let product = ProductId::parse("ordinary-product").unwrap();
    let initiative = InitiativeId::parse("ordinary-initiative").unwrap();
    let roadmap = RoadmapId::parse("ordinary-roadmap").unwrap();
    let kpi = KpiId::parse("ordinary-kpi").unwrap();
    let project = ProjectId::parse("ordinary-project").unwrap();
    let milestone = MilestoneId::parse("ordinary-milestone").unwrap();
    let stakeholder = StakeholderId::parse("ordinary-stakeholder").unwrap();
    let mut s = InMemoryRelationshipService::with_execution_authorities(
        FixedClock,
        InMemoryEndpointCatalog::new([
            EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
                portfolio.clone(),
                AggregateVersion::initial(),
                DataClassification::Public,
            )),
            EndpointSnapshot::Product(ProductSnapshot::new(
                product.clone(),
                AggregateVersion::initial(),
                DataClassification::Internal,
            )),
            EndpointSnapshot::Initiative(pmc_domain::relationships::InitiativeSnapshot::new(
                initiative.clone(),
                AggregateVersion::initial(),
                DataClassification::Confidential,
            )),
            EndpointSnapshot::Roadmap(pmc_domain::relationships::RoadmapSnapshot::new(
                roadmap.clone(),
                AggregateVersion::initial(),
                DataClassification::Internal,
            )),
            EndpointSnapshot::Kpi(pmc_domain::relationships::KpiSnapshot::new(
                kpi.clone(),
                AggregateVersion::initial(),
                DataClassification::Internal,
            )),
            EndpointSnapshot::Project(pmc_domain::relationships::ProjectSnapshot::new(
                project.clone(),
                AggregateVersion::initial(),
                DataClassification::Confidential,
            )),
            EndpointSnapshot::Milestone(pmc_domain::relationships::MilestoneSnapshot::new(
                milestone.clone(),
                project.clone(),
                AggregateVersion::initial(),
                DataClassification::Restricted,
            )),
        ]),
        AuditIds::default(),
        ExecIds(700),
        AllowRemoval,
        AllowApproval,
    );
    s.create_stakeholder(CreateStakeholder {
        id: stakeholder.clone(),
        name: pmc_domain::relationships::StakeholderName::parse("Synthetic owner").unwrap(),
        kind: StakeholderKind::Person,
        classification: Some(DataClassification::Internal),
        provenance: pmc_domain::provenance::Provenance::SyntheticFixture(
            pmc_domain::provenance::ProvenanceReference::parse("ordinary-fixture").unwrap(),
        ),
        context: ctx("ordinary-create-stakeholder"),
    })
    .unwrap();
    s.link_portfolio_product(LinkPortfolioProduct {
        id: RelationshipId::parse("ordinary-rel-portfolio-product").unwrap(),
        portfolio_id: portfolio.clone(),
        product_id: product.clone(),
        expected_portfolio_version: AggregateVersion::initial(),
        expected_product_version: AggregateVersion::initial(),
        context: ctx("ordinary-link-portfolio-product"),
    })
    .unwrap();
    s.link_portfolio_initiative(LinkPortfolioInitiative {
        id: RelationshipId::parse("ordinary-rel-portfolio-initiative").unwrap(),
        portfolio_id: portfolio.clone(),
        initiative_id: initiative.clone(),
        expected_portfolio_version: AggregateVersion::initial(),
        expected_initiative_version: AggregateVersion::initial(),
        context: ctx("ordinary-link-portfolio-initiative"),
    })
    .unwrap();
    s.link_product_roadmap(LinkProductRoadmap {
        id: RelationshipId::parse("ordinary-rel-product-roadmap").unwrap(),
        product_id: product.clone(),
        roadmap_id: roadmap.clone(),
        expected_product_version: AggregateVersion::initial(),
        expected_roadmap_version: AggregateVersion::initial(),
        context: ctx("ordinary-link-product-roadmap"),
    })
    .unwrap();
    s.link_product_kpi(LinkProductKpi {
        id: RelationshipId::parse("ordinary-rel-product-kpi").unwrap(),
        product_id: product.clone(),
        kpi_id: kpi.clone(),
        expected_product_version: AggregateVersion::initial(),
        expected_kpi_version: AggregateVersion::initial(),
        context: ctx("ordinary-link-product-kpi"),
    })
    .unwrap();
    s.link_initiative_project(LinkInitiativeProject {
        id: RelationshipId::parse("ordinary-rel-initiative-project").unwrap(),
        initiative_id: initiative.clone(),
        project_id: project.clone(),
        expected_initiative_version: AggregateVersion::initial(),
        expected_project_version: AggregateVersion::initial(),
        context: ctx("ordinary-link-initiative-project"),
    })
    .unwrap();
    s.link_project_product(LinkProjectProduct {
        id: RelationshipId::parse("ordinary-rel-project-product").unwrap(),
        project_id: project.clone(),
        product_id: product.clone(),
        expected_project_version: AggregateVersion::initial(),
        expected_product_version: AggregateVersion::initial(),
        context: ctx("ordinary-link-project-product"),
    })
    .unwrap();
    s.link_stakeholder_relationship(LinkStakeholderRelationship {
        id: RelationshipId::parse("ordinary-rel-stakeholder-responsibility").unwrap(),
        stakeholder_id: stakeholder.clone(),
        subject: StakeholderSubject::Product(product.clone()),
        purpose: StakeholderRelationshipPurpose::Responsibility,
        expected_stakeholder_version: AggregateVersion::initial(),
        expected_subject_version: AggregateVersion::initial(),
        context: ctx("ordinary-link-stakeholder-responsibility"),
    })
    .unwrap();
    s.link_stakeholder_relationship(LinkStakeholderRelationship {
        id: RelationshipId::parse("ordinary-rel-stakeholder-dependency").unwrap(),
        stakeholder_id: stakeholder.clone(),
        subject: StakeholderSubject::Initiative(initiative.clone()),
        purpose: StakeholderRelationshipPurpose::Dependency,
        expected_stakeholder_version: AggregateVersion::initial(),
        expected_subject_version: AggregateVersion::initial(),
        context: ctx("ordinary-link-stakeholder-dependency"),
    })
    .unwrap();
    s.link_stakeholder_relationship(LinkStakeholderRelationship {
        id: RelationshipId::parse("ordinary-rel-stakeholder-milestone").unwrap(),
        stakeholder_id: stakeholder.clone(),
        subject: StakeholderSubject::Milestone(milestone),
        purpose: StakeholderRelationshipPurpose::Responsibility,
        expected_stakeholder_version: AggregateVersion::initial(),
        expected_subject_version: AggregateVersion::initial(),
        context: ctx("ordinary-link-stakeholder-milestone"),
    })
    .unwrap();
    s.update_stakeholder(UpdateStakeholderDetails {
        id: stakeholder,
        expected_version: AggregateVersion::initial(),
        name: pmc_domain::relationships::StakeholderName::parse("Synthetic owner").unwrap(),
        classification: Some(DataClassification::Restricted),
        context: ctx("ordinary-update-stakeholder"),
    })
    .unwrap();
    s
}
fn execute(
    prepared: &PreparedIntent,
    key: &str,
    confirmation: &str,
) -> ApproveAndExecuteRemoveRelationship {
    ApproveAndExecuteRemoveRelationship {
        prepared_id: prepared.id().clone(),
        actor: AuditActor::HeadOfProducts,
        confirmation: confirmation.to_owned(),
        acknowledged_payload_digest: prepared.payload_digest().clone(),
        context: ctx(key),
    }
}
fn ver(v: i64) -> AggregateVersion {
    AggregateVersion::new(v as u64).unwrap()
}
fn ts(v: i64) -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(v)
}
fn cls(v: &str) -> DataClassification {
    DataClassification::from_persisted(v).unwrap()
}
fn rel_kind(v: &str) -> RelationshipKind {
    RelationshipKind::from_persisted(v).unwrap()
}
fn rel_purpose(v: Option<&str>) -> Option<StakeholderRelationshipPurpose> {
    match v {
        None | Some("none") => None,
        Some(value) => Some(StakeholderRelationshipPurpose::from_persisted(value).unwrap()),
    }
}
fn error_code(v: &str) -> ErrorCode {
    match v {
        "VALIDATION_INVALID_FIELD" => ErrorCode::ValidationInvalidField,
        "DOMAIN_CONFLICT" => ErrorCode::DomainConflict,
        "DOMAIN_NOT_FOUND" => ErrorCode::DomainNotFound,
        "SECURITY_POLICY_DENIED" => ErrorCode::SecurityPolicyDenied,
        "AI_POLICY_DENIED" => ErrorCode::AiPolicyDenied,
        "SECURITY_PREVIEW_EXPIRED_OR_CHANGED" => ErrorCode::SecurityPreviewExpiredOrChanged,
        "DOMAIN_IDEMPOTENCY_CONFLICT" => ErrorCode::DomainIdempotencyConflict,
        "PLATFORM_INTERNAL" => ErrorCode::PlatformInternal,
        _ => panic!("error code"),
    }
}
fn ep_type(e: &EndpointSnapshot) -> &'static str {
    match e {
        EndpointSnapshot::Portfolio(_) => "portfolio",
        EndpointSnapshot::Product(_) => "product",
        EndpointSnapshot::Initiative(_) => "initiative",
        EndpointSnapshot::Roadmap(_) => "roadmap",
        EndpointSnapshot::Kpi(_) => "kpi",
        EndpointSnapshot::Project(_) => "project",
        EndpointSnapshot::Milestone(_) => "milestone",
        EndpointSnapshot::Stakeholder(_) => "stakeholder",
    }
}
fn aggregate_ep_type(e: &EndpointSnapshot) -> &'static str {
    match e {
        EndpointSnapshot::Kpi(_) => "kpi_definition",
        _ => ep_type(e),
    }
}
fn ep_id(e: &EndpointSnapshot) -> &str {
    match e {
        EndpointSnapshot::Portfolio(v) => v.id().as_str(),
        EndpointSnapshot::Product(v) => v.id().as_str(),
        EndpointSnapshot::Initiative(v) => v.id().as_str(),
        EndpointSnapshot::Roadmap(v) => v.id().as_str(),
        EndpointSnapshot::Kpi(v) => v.id().as_str(),
        EndpointSnapshot::Project(v) => v.id().as_str(),
        EndpointSnapshot::Milestone(v) => v.id().as_str(),
        EndpointSnapshot::Stakeholder(v) => v.id().as_str(),
    }
}
fn ep_ver(e: &EndpointSnapshot) -> i64 {
    match e {
        EndpointSnapshot::Portfolio(v) => v.version().get() as i64,
        EndpointSnapshot::Product(v) => v.version().get() as i64,
        EndpointSnapshot::Initiative(v) => v.version().get() as i64,
        EndpointSnapshot::Roadmap(v) => v.version().get() as i64,
        EndpointSnapshot::Kpi(v) => v.version().get() as i64,
        EndpointSnapshot::Project(v) => v.version().get() as i64,
        EndpointSnapshot::Milestone(v) => v.version().get() as i64,
        EndpointSnapshot::Stakeholder(v) => v.version().get() as i64,
    }
}
fn ep_cls(e: &EndpointSnapshot) -> &'static str {
    match e {
        EndpointSnapshot::Portfolio(v) => v.classification().as_persisted(),
        EndpointSnapshot::Product(v) => v.classification().as_persisted(),
        EndpointSnapshot::Initiative(v) => v.classification().as_persisted(),
        EndpointSnapshot::Roadmap(v) => v.classification().as_persisted(),
        EndpointSnapshot::Kpi(v) => v.classification().as_persisted(),
        EndpointSnapshot::Project(v) => v.classification().as_persisted(),
        EndpointSnapshot::Milestone(v) => v.classification().as_persisted(),
        EndpointSnapshot::Stakeholder(v) => v.classification().as_persisted(),
    }
}
fn ep_parent(e: &EndpointSnapshot) -> Option<&str> {
    match e {
        EndpointSnapshot::Milestone(v) => Some(v.project_id().as_str()),
        _ => None,
    }
}
fn ep_row(r: &Row<'_>) -> rusqlite::Result<EndpointSnapshot> {
    let t: String = r.get(0)?;
    let id: String = r.get(1)?;
    let v = ver(r.get(2)?);
    let c = cls(&r.get::<_, String>(3)?);
    let parent: Option<String> = r.get(4)?;
    Ok(match t.as_str() {
        "portfolio" => EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
            PortfolioId::parse(id).unwrap(),
            v,
            c,
        )),
        "product" => {
            EndpointSnapshot::Product(ProductSnapshot::new(ProductId::parse(id).unwrap(), v, c))
        }
        "initiative" => {
            EndpointSnapshot::Initiative(pmc_domain::relationships::InitiativeSnapshot::new(
                InitiativeId::parse(id).unwrap(),
                v,
                c,
            ))
        }
        "roadmap" => EndpointSnapshot::Roadmap(pmc_domain::relationships::RoadmapSnapshot::new(
            RoadmapId::parse(id).unwrap(),
            v,
            c,
        )),
        "kpi" | "kpi_definition" => EndpointSnapshot::Kpi(
            pmc_domain::relationships::KpiSnapshot::new(KpiId::parse(id).unwrap(), v, c),
        ),
        "project" => EndpointSnapshot::Project(pmc_domain::relationships::ProjectSnapshot::new(
            ProjectId::parse(id).unwrap(),
            v,
            c,
        )),
        "milestone" => {
            EndpointSnapshot::Milestone(pmc_domain::relationships::MilestoneSnapshot::new(
                MilestoneId::parse(id).unwrap(),
                ProjectId::parse(parent.unwrap()).unwrap(),
                v,
                c,
            ))
        }
        "stakeholder" => {
            EndpointSnapshot::Stakeholder(pmc_domain::relationships::StakeholderSnapshot::new(
                StakeholderId::parse(id).unwrap(),
                v,
                c,
            ))
        }
        _ => panic!("endpoint type"),
    })
}
fn persist(c: &mut Connection, s: &RelationshipH2bPersistenceSnapshot) {
    let tx = c.transaction().unwrap();
    let rel = s
        .relationships()
        .first()
        .or_else(|| s.ordinary_history_relationships().first())
        .expect("current or historical relationship fixture");
    for e in rel.endpoints() {
        tx.execute("INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES(?1,?2,?3,?4,1000,1000)",params![ep_id(e),ep_type(e),ep_ver(e),ep_cls(e)]).unwrap();
    }
    for r in s.relationships() {
        tx.execute("INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES(?1,'relationship',?2,?3,?4,?5)",params![r.id().as_str(),r.version().get() as i64,r.classification().as_persisted(),r.created_at().unix_millis(),r.updated_at().unix_millis()]).unwrap();
        tx.execute(
            "INSERT INTO relationships(id,kind,purpose) VALUES(?1,'portfolio_product',NULL)",
            params![r.id().as_str()],
        )
        .unwrap();
        for (i, e) in r.endpoints().iter().enumerate() {
            tx.execute("INSERT INTO relationship_endpoints(relationship_id,ordinal,target_type,target_id,target_version,target_classification,parent_project_id) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![r.id().as_str(),i as i64,aggregate_ep_type(e),ep_id(e),ep_ver(e),ep_cls(e),ep_parent(e)]).unwrap();
        }
    }
    for a in s.audits() {
        let (target_type, target_id) = match a.target() {
            AuditTarget::Portfolio(v) => ("portfolio", v.as_str()),
            AuditTarget::Product(v) => ("product", v.as_str()),
            AuditTarget::Relationship(v) => ("relationship", v.as_str()),
            _ => panic!("audit target"),
        };
        tx.execute("INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![a.id().as_str(),a.occurred_at().unix_millis(),a.actor().as_persisted(),a.module().as_persisted(),a.code().as_str(),target_type,target_id,a.correlation_id().as_str(),a.policy_outcome().as_persisted(),a.approval_outcome().as_persisted(),a.execution_outcome().as_persisted(),a.effect_scope().as_persisted()]).unwrap();
        for (i, e) in a.actual_effects().iter().enumerate() {
            tx.execute("INSERT INTO audit_effects(audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES(?1,?2,?3,?4,?5,?6)",params![a.id().as_str(),i as i64,e.as_str(),a.effect_scope().as_persisted(),target_type,target_id]).unwrap();
        }
    }
    for x in s.ordinary_replay() {
        let RelationshipPersistenceCommand::Link {
            kind: k,
            endpoints,
            expected_versions,
            purpose: p,
            ..
        } = x.command()
        else {
            panic!("ordinary command")
        };
        let RelationshipPersistenceResult::Relationship(o) = x.result() else {
            panic!("ordinary result")
        };
        tx.execute("INSERT INTO relationship_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES(?1,'link',?2,?3,'relationship',?4)",params![x.idempotency_id().as_str(),x.correlation_id().as_str(),x.operation_ordinal()as i64,o.value().id().as_str()]).unwrap();
        tx.execute("INSERT INTO relationship_link_command_results(idempotency_id,relationship_id,relationship_kind,purpose,result_version,result_classification,result_created_at,result_updated_at,effect_scope) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![x.idempotency_id().as_str(),o.value().id().as_str(),match k{RelationshipKind::PortfolioProduct=>"portfolio_product",_=>panic!("kind")},p.map(|v|match v{StakeholderRelationshipPurpose::Responsibility=>"responsibility",StakeholderRelationshipPurpose::Dependency=>"dependency"}),o.value().version().get()as i64,o.value().classification().as_persisted(),o.value().created_at().unix_millis(),o.value().updated_at().unix_millis(),o.outcome().effect_scope().as_persisted()]).unwrap();
        for (i, e) in endpoints.iter().enumerate() {
            tx.execute("INSERT INTO relationship_link_command_endpoints(idempotency_id,ordinal,endpoint_type,endpoint_id,expected_version,snapshot_version,classification,parent_project_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![x.idempotency_id().as_str(),i as i64,ep_type(e),ep_id(e),expected_versions[i].get()as i64,ep_ver(e),ep_cls(e),ep_parent(e)]).unwrap();
            tx.execute("INSERT INTO relationship_link_result_endpoints(idempotency_id,ordinal,endpoint_type,endpoint_id,endpoint_version,classification,parent_project_id) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![x.idempotency_id().as_str(),i as i64,ep_type(e),ep_id(e),ep_ver(e),ep_cls(e),ep_parent(e)]).unwrap();
        }
        for (i, a) in x.audit_event_ids().iter().enumerate() {
            tx.execute("INSERT INTO relationship_replay_audits(idempotency_id,ordinal,audit_event_id,correlation_id) VALUES(?1,?2,?3,?4)",params![x.idempotency_id().as_str(),i as i64,a.as_str(),x.correlation_id().as_str()]).unwrap();
        }
    }
    for x in s.h2b_replay() {
        let RelationshipH2bPersistenceCommand::Prepare { relationship_id } = x.command() else {
            continue;
        };
        let RelationshipH2bPersistenceResult::Prepared(p) = x.result() else {
            continue;
        };
        tx.execute("INSERT INTO relationship_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES(?1,'prepare_remove',?2,?3,'prepared',?4)",params![x.idempotency_id().as_str(),x.correlation_id().as_str(),x.operation_ordinal()as i64,p.id().as_str()]).unwrap();
        tx.execute("INSERT INTO relationship_h2b_command_results(idempotency_id,command_kind,result_kind,relationship_id,result_prepared_intent_id) VALUES(?1,'prepare_remove','prepared',?2,?3)",params![x.idempotency_id().as_str(),relationship_id.as_str(),p.id().as_str()]).unwrap();
        for (i, a) in x.audit_event_ids().iter().enumerate() {
            tx.execute("INSERT INTO relationship_replay_audits(idempotency_id,ordinal,audit_event_id,correlation_id) VALUES(?1,?2,?3,?4)",params![x.idempotency_id().as_str(),i as i64,a.as_str(),x.correlation_id().as_str()]).unwrap();
        }
        let pview = p.preview();
        tx.execute("INSERT INTO prepared_intents(id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,confirmation_challenge,expires_at,created_at) VALUES(?1,?2,'relationship.remove',?3,?4,'allowed','not_cancellable_after_submit','head_of_products',?5,?6,1000)",params![p.id().as_str(),pview.intent_version()as i64,p.payload_digest().as_str(),pview.classification().as_persisted(),pview.confirmation_challenge(),pview.expires_at().unix_millis()]).unwrap();
        tx.execute("INSERT INTO prepared_removal_payloads(prepared_intent_id,relationship_id,relationship_version,relationship_kind,purpose) VALUES(?1,?2,?3,'portfolio_product','none')",params![p.id().as_str(),pview.relationship_id().as_str(),pview.relationship_version().get()as i64]).unwrap();
        for (i, e) in pview.endpoints().iter().enumerate() {
            tx.execute("INSERT INTO prepared_removal_endpoints(prepared_intent_id,ordinal,endpoint_type,endpoint_id,endpoint_version,classification,parent_project_id) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![p.id().as_str(),i as i64,ep_type(e),ep_id(e),ep_ver(e),ep_cls(e),ep_parent(e)]).unwrap();
        }
        let ev = pview.evidence();
        tx.execute("INSERT INTO prepared_intent_recovery_evidence(prepared_intent_id,recovery_evidence_id,name,verified_at,relationship_id,compatible) VALUES(?1,?2,?3,?4,?5,?6)",params![p.id().as_str(),ev.id().as_str(),ev.name(),ev.verified_at().unix_millis(),ev.relationship_id().as_str(),i64::from(ev.compatible())]).unwrap();
        for (i, e) in pview.effects().iter().enumerate() {
            tx.execute("INSERT INTO prepared_intent_effects(prepared_intent_id,ordinal,effect_code,target_type,target_id) VALUES(?1,?2,?3,'relationship',?4)",params![p.id().as_str(),i as i64,match e{RemovalEffect::RemoveRelationshipRecord=>"remove_relationship_record",RemovalEffect::RemoveSemanticRelationshipIndex=>"remove_semantic_relationship_index",RemovalEffect::CreateIdempotencyTombstone=>"create_idempotency_tombstone"},pview.relationship_id().as_str()]).unwrap();
        }
    }
    for x in s.h2b_replay() {
        let (
            command_kind,
            result_kind,
            prepared_id,
            actor,
            acknowledged,
            confirmation,
            result_relationship,
            error_fields,
        ) = match (x.command(), x.result()) {
            (
                RelationshipH2bPersistenceCommand::Cancel { prepared_id, actor },
                RelationshipH2bPersistenceResult::Cancelled,
            ) => (
                "cancel_remove",
                "cancelled",
                Some(prepared_id.as_str()),
                Some(actor.as_persisted()),
                None,
                None,
                None,
                None,
            ),
            (
                RelationshipH2bPersistenceCommand::Execute {
                    prepared_id,
                    actor,
                    acknowledged_payload_digest,
                    confirmation_digest,
                },
                RelationshipH2bPersistenceResult::Removal(outcome),
            ) => (
                "execute_remove",
                "removal",
                Some(prepared_id.as_str()),
                Some(actor.as_persisted()),
                Some(acknowledged_payload_digest.as_str()),
                Some(confirmation_digest.as_str()),
                Some(outcome.relationship_id.as_str()),
                None,
            ),
            (
                RelationshipH2bPersistenceCommand::Execute {
                    prepared_id,
                    actor,
                    acknowledged_payload_digest,
                    confirmation_digest,
                },
                RelationshipH2bPersistenceResult::Rejection(error),
            ) => (
                "execute_remove",
                "rejection",
                Some(prepared_id.as_str()),
                Some(actor.as_persisted()),
                Some(acknowledged_payload_digest.as_str()),
                Some(confirmation_digest.as_str()),
                None,
                Some((
                    error.code().as_str(),
                    error.message_key().as_str(),
                    i64::from(error.retryable()),
                )),
            ),
            _ => continue,
        };
        let result_reference = result_relationship.or(prepared_id);
        let (error_code, error_message_key, error_retryable) = error_fields
            .map_or((None, None, None), |(code, key, retryable)| {
                (Some(code), Some(key), Some(retryable))
            });
        tx.execute("INSERT INTO relationship_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference,error_code,error_message_key,error_retryable) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)", params![x.idempotency_id().as_str(), command_kind, x.correlation_id().as_str(), x.operation_ordinal() as i64, result_kind, result_reference, error_code, error_message_key, error_retryable]).unwrap();
        tx.execute("INSERT INTO relationship_h2b_command_results(idempotency_id,command_kind,result_kind,prepared_intent_id,actor,acknowledged_payload_digest,confirmation_digest,result_relationship_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)", params![x.idempotency_id().as_str(), command_kind, result_kind, prepared_id, actor, acknowledged, confirmation, result_relationship]).unwrap();
        for (i, audit_id) in x.audit_event_ids().iter().enumerate() {
            tx.execute("INSERT INTO relationship_replay_audits(idempotency_id,ordinal,audit_event_id,correlation_id) VALUES(?1,?2,?3,?4)", params![x.idempotency_id().as_str(), i as i64, audit_id.as_str(), x.correlation_id().as_str()]).unwrap();
        }
    }
    for id in s.tombstoned_ordinary() {
        let prepared = s
            .h2b_replay()
            .iter()
            .find_map(|x| match x.result() {
                RelationshipH2bPersistenceResult::Prepared(p) => Some(p.id().as_str()),
                _ => None,
            })
            .expect("terminal prepared intent");
        tx.execute("INSERT INTO relationship_replay_tombstones(idempotency_id,kind,removal_prepared_intent_id) VALUES(?1,'ordinary',?2)", params![id.as_str(), prepared]).unwrap();
    }
    for id in s.tombstoned_h2b() {
        let prepared = s
            .h2b_replay()
            .iter()
            .find_map(|x| match x.result() {
                RelationshipH2bPersistenceResult::Prepared(p) => Some(p.id().as_str()),
                _ => None,
            })
            .expect("terminal prepared intent");
        tx.execute("INSERT INTO relationship_replay_tombstones(idempotency_id,kind,removal_prepared_intent_id) VALUES(?1,'h2b',?2)", params![id.as_str(), prepared]).unwrap();
    }
    tx.commit().unwrap();
}

fn rel_kind_name(k: RelationshipKind) -> &'static str {
    k.as_persisted()
}
fn purpose_name(p: StakeholderRelationshipPurpose) -> &'static str {
    p.as_persisted()
}
fn persist_audit(tx: &rusqlite::Transaction<'_>, a: &AuditEvent) {
    let (target_type, target_id) = match a.target() {
        AuditTarget::Portfolio(v) => ("portfolio", v.as_str()),
        AuditTarget::Product(v) => ("product", v.as_str()),
        AuditTarget::Initiative(v) => ("initiative", v.as_str()),
        AuditTarget::Roadmap(v) => ("roadmap", v.as_str()),
        AuditTarget::Kpi(v) => ("kpi_definition", v.as_str()),
        AuditTarget::Project(v) => ("project", v.as_str()),
        AuditTarget::Milestone(v) => ("milestone", v.as_str()),
        AuditTarget::Stakeholder(v) => ("stakeholder", v.as_str()),
        AuditTarget::Relationship(v) => ("relationship", v.as_str()),
        _ => panic!("ordinary audit target"),
    };
    tx.execute("INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![a.id().as_str(),a.occurred_at().unix_millis(),a.actor().as_persisted(),a.module().as_persisted(),a.code().as_str(),target_type,target_id,a.correlation_id().as_str(),a.policy_outcome().as_persisted(),a.approval_outcome().as_persisted(),a.execution_outcome().as_persisted(),a.effect_scope().as_persisted()]).unwrap();
    for (i, e) in a.actual_effects().iter().enumerate() {
        tx.execute("INSERT INTO audit_effects(audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES(?1,?2,?3,?4,?5,?6)",params![a.id().as_str(),i as i64,e.as_str(),a.effect_scope().as_persisted(),target_type,target_id]).unwrap();
    }
}
fn persist_link_replay(
    tx: &rusqlite::Transaction<'_>,
    capsule: &RelationshipReplayCapsule,
    outcome: &pmc_domain::relationships::MutationOutcome<RelationshipRecord>,
    kind: RelationshipKind,
    endpoints: &[EndpointSnapshot],
    expected: &[AggregateVersion],
    purpose: Option<StakeholderRelationshipPurpose>,
) {
    let r = outcome.value();
    tx.execute("INSERT INTO relationship_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES(?1,'link',?2,?3,'relationship',?4)",params![capsule.idempotency_id().as_str(),capsule.correlation_id().as_str(),capsule.operation_ordinal()as i64,r.id().as_str()]).unwrap();
    tx.execute("INSERT INTO relationship_link_command_results(idempotency_id,relationship_id,relationship_kind,purpose,result_version,result_classification,result_created_at,result_updated_at,effect_scope) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![capsule.idempotency_id().as_str(),r.id().as_str(),rel_kind_name(kind),purpose.map(purpose_name),r.version().get()as i64,r.classification().as_persisted(),r.created_at().unix_millis(),r.updated_at().unix_millis(),outcome.outcome().effect_scope().as_persisted()]).unwrap();
    for (i, e) in endpoints.iter().enumerate() {
        tx.execute("INSERT INTO relationship_link_command_endpoints(idempotency_id,ordinal,endpoint_type,endpoint_id,expected_version,snapshot_version,classification,parent_project_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![capsule.idempotency_id().as_str(),i as i64,ep_type(e),ep_id(e),expected[i].get()as i64,ep_ver(e),ep_cls(e),ep_parent(e)]).unwrap();
        tx.execute("INSERT INTO relationship_link_result_endpoints(idempotency_id,ordinal,endpoint_type,endpoint_id,endpoint_version,classification,parent_project_id) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![capsule.idempotency_id().as_str(),i as i64,ep_type(e),ep_id(e),ep_ver(e),ep_cls(e),ep_parent(e)]).unwrap();
    }
}
fn persist_ordinary(c: &mut Connection, s: &RelationshipPersistenceSnapshot) {
    let tx = c.transaction().unwrap();
    for relationship in s.relationships() {
        for endpoint in relationship.endpoints() {
            if !matches!(endpoint, EndpointSnapshot::Stakeholder(_)) {
                tx.execute("INSERT OR IGNORE INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES(?1,?2,?3,?4,1000,1000)", params![ep_id(endpoint), aggregate_ep_type(endpoint), ep_ver(endpoint), ep_cls(endpoint)]).unwrap();
            }
        }
    }
    if s.relationships().iter().any(|relationship| {
        relationship
            .endpoints()
            .iter()
            .any(|endpoint| matches!(endpoint, EndpointSnapshot::Milestone(_)))
    }) {
        tx.execute("INSERT OR IGNORE INTO projects(id,name,start_at,end_at,provenance_kind,provenance_reference) VALUES('ordinary-project','Synthetic project',1000,2000,'synthetic_fixture','ordinary-fixture')", []).unwrap();
        tx.execute("INSERT OR IGNORE INTO milestones(id,project_id,name,verification_criteria,due_at,provenance_kind,provenance_reference) VALUES('ordinary-milestone','ordinary-project','Synthetic milestone','Synthetic verification',2000,'synthetic_fixture','ordinary-fixture')", []).unwrap();
    }
    for stakeholder in s.stakeholders() {
        tx.execute("INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES(?1,'stakeholder',?2,?3,?4,?5)", params![stakeholder.id().as_str(), stakeholder.version().get() as i64, stakeholder.classification().as_persisted(), stakeholder.created_at().unix_millis(), stakeholder.updated_at().unix_millis()]).unwrap();
        let (pk, pref) = match stakeholder.provenance() {
            pmc_domain::provenance::Provenance::UserEntered => ("user_entered", None),
            pmc_domain::provenance::Provenance::AuthoritativeTransition(v) => {
                ("authoritative_transition", Some(v.as_str()))
            }
            pmc_domain::provenance::Provenance::SyntheticFixture(v) => {
                ("synthetic_fixture", Some(v.as_str()))
            }
        };
        tx.execute("INSERT INTO stakeholders(id,name,kind,provenance_kind,provenance_reference) VALUES(?1,?2,?3,?4,?5)", params![stakeholder.id().as_str(), stakeholder.name().as_str(), stakeholder.kind().as_persisted(), pk, pref]).unwrap();
    }
    for relationship in s.relationships() {
        tx.execute("INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES(?1,'relationship',?2,?3,?4,?5)", params![relationship.id().as_str(), relationship.version().get() as i64, relationship.classification().as_persisted(), relationship.created_at().unix_millis(), relationship.updated_at().unix_millis()]).unwrap();
        tx.execute(
            "INSERT INTO relationships(id,kind,purpose) VALUES(?1,?2,?3)",
            params![
                relationship.id().as_str(),
                rel_kind_name(relationship.kind()),
                relationship.purpose().map(purpose_name)
            ],
        )
        .unwrap();
        for (i, e) in relationship.endpoints().iter().enumerate() {
            tx.execute("INSERT INTO relationship_endpoints(relationship_id,ordinal,target_type,target_id,target_version,target_classification,parent_project_id) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![relationship.id().as_str(),i as i64,aggregate_ep_type(e),ep_id(e),ep_ver(e),ep_cls(e),ep_parent(e)]).unwrap();
        }
    }
    for a in s.audits() {
        persist_audit(&tx, a);
    }
    for capsule in s.replay() {
        match (capsule.command(), capsule.result()) {
            (
                RelationshipPersistenceCommand::CreateStakeholder {
                    id,
                    name,
                    kind,
                    classification,
                    provenance,
                },
                RelationshipPersistenceResult::Stakeholder(outcome),
            ) => {
                let (pk, pref) = match provenance {
                    pmc_domain::provenance::Provenance::UserEntered => ("user_entered", None),
                    pmc_domain::provenance::Provenance::AuthoritativeTransition(v) => {
                        ("authoritative_transition", Some(v.as_str()))
                    }
                    pmc_domain::provenance::Provenance::SyntheticFixture(v) => {
                        ("synthetic_fixture", Some(v.as_str()))
                    }
                };
                let result = outcome.value();
                tx.execute("INSERT INTO relationship_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES(?1,'create_stakeholder',?2,?3,'stakeholder',?4)",params![capsule.idempotency_id().as_str(),capsule.correlation_id().as_str(),capsule.operation_ordinal() as i64,result.id().as_str()]).unwrap();
                tx.execute("INSERT INTO relationship_stakeholder_command_results(idempotency_id,command_kind,command_id,result_id,command_name,command_stakeholder_kind,command_classification,command_provenance_kind,command_provenance_reference,result_name,result_stakeholder_kind,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at,effect_scope) VALUES(?1,'create_stakeholder',?2,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",params![capsule.idempotency_id().as_str(),id.as_str(),name.as_str(),kind.as_persisted(),classification.map(|v|v.as_persisted()),pk,pref,result.name().as_str(),result.kind().as_persisted(),result.classification().as_persisted(),pk,result.provenance().reference().map(|v|v.as_str()),result.version().get() as i64,result.created_at().unix_millis(),result.updated_at().unix_millis(),outcome.outcome().effect_scope().as_persisted()]).unwrap();
            }
            (
                RelationshipPersistenceCommand::UpdateStakeholder {
                    id,
                    expected_version,
                    name,
                    classification,
                },
                RelationshipPersistenceResult::Stakeholder(outcome),
            ) => {
                let result = outcome.value();
                tx.execute("INSERT INTO relationship_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES(?1,'update_stakeholder',?2,?3,'stakeholder',?4)",params![capsule.idempotency_id().as_str(),capsule.correlation_id().as_str(),capsule.operation_ordinal() as i64,result.id().as_str()]).unwrap();
                tx.execute("INSERT INTO relationship_stakeholder_command_results(idempotency_id,command_kind,command_id,result_id,command_expected_version,command_name,command_classification,result_name,result_stakeholder_kind,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at,effect_scope) VALUES(?1,'update_stakeholder',?2,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",params![capsule.idempotency_id().as_str(),id.as_str(),expected_version.get() as i64,name.as_str(),classification.map(|v|v.as_persisted()),result.name().as_str(),result.kind().as_persisted(),result.classification().as_persisted(),match result.provenance(){pmc_domain::provenance::Provenance::UserEntered=>"user_entered",pmc_domain::provenance::Provenance::AuthoritativeTransition(_)=>"authoritative_transition",pmc_domain::provenance::Provenance::SyntheticFixture(_)=>"synthetic_fixture"},result.provenance().reference().map(|v|v.as_str()),result.version().get() as i64,result.created_at().unix_millis(),result.updated_at().unix_millis(),outcome.outcome().effect_scope().as_persisted()]).unwrap();
            }
            (
                RelationshipPersistenceCommand::Link {
                    id: _,
                    kind,
                    endpoints,
                    expected_versions,
                    purpose,
                },
                RelationshipPersistenceResult::Relationship(outcome),
            ) => {
                persist_link_replay(
                    &tx,
                    capsule,
                    outcome,
                    *kind,
                    endpoints,
                    expected_versions,
                    *purpose,
                );
            }
            _ => panic!("ordinary command/result mismatch"),
        }
        for (i, a) in capsule.audit_event_ids().iter().enumerate() {
            tx.execute("INSERT INTO relationship_replay_audits(idempotency_id,ordinal,audit_event_id,correlation_id) VALUES(?1,?2,?3,?4)",params![capsule.idempotency_id().as_str(),i as i64,a.as_str(),capsule.correlation_id().as_str()]).unwrap();
        }
    }
    tx.commit().unwrap();
}

fn decode_prepared(c: &Connection, id: &str) -> PreparedIntent {
    let(rid,rv,k,p):(String,i64,String,String)=c.query_row("SELECT relationship_id,relationship_version,relationship_kind,purpose FROM prepared_removal_payloads WHERE prepared_intent_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    let mut q=c.prepare("SELECT endpoint_type,endpoint_id,endpoint_version,classification,parent_project_id FROM prepared_removal_endpoints WHERE prepared_intent_id=?1 ORDER BY ordinal").unwrap();
    let es = q
        .query_map([id], |r| {
            let t: String = r.get(0)?;
            let i: String = r.get(1)?;
            let v: i64 = r.get(2)?;
            let cl: String = r.get(3)?;
            let parent: Option<String> = r.get(4)?;
            Ok((t, i, v, cl, parent))
        })
        .unwrap()
        .map(|x| {
            let (t, i, v, cl, parent) = x.unwrap();
            match t.as_str() {
                "portfolio" => EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
                    PortfolioId::parse(i).unwrap(),
                    ver(v),
                    cls(&cl),
                )),
                "product" => EndpointSnapshot::Product(ProductSnapshot::new(
                    ProductId::parse(i).unwrap(),
                    ver(v),
                    cls(&cl),
                )),
                "milestone" => {
                    EndpointSnapshot::Milestone(pmc_domain::relationships::MilestoneSnapshot::new(
                        MilestoneId::parse(i).unwrap(),
                        ProjectId::parse(parent.unwrap()).unwrap(),
                        ver(v),
                        cls(&cl),
                    ))
                }
                _ => panic!("endpoint"),
            }
        })
        .collect();
    let(d,cl,ch,ex):(String,String,String,i64)=c.query_row("SELECT payload_digest,classification,confirmation_challenge,expires_at FROM prepared_intents WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    let(ei,en,vt,co):(String,String,i64,i64)=c.query_row("SELECT recovery_evidence_id,name,verified_at,compatible FROM prepared_intent_recovery_evidence WHERE prepared_intent_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    let ev = RecoveryEvidence::new(
        RecoveryEvidenceId::parse(ei).unwrap(),
        en,
        ts(vt),
        RelationshipId::parse(rid.clone()).unwrap(),
        co != 0,
    )
    .unwrap();
    let mut effects = Vec::new();
    let mut effect_q = c
        .prepare("SELECT effect_code FROM prepared_intent_effects WHERE prepared_intent_id=?1 ORDER BY ordinal")
        .unwrap();
    for value in effect_q
        .query_map([id], |row| row.get::<_, String>(0))
        .unwrap()
    {
        effects.push(match value.unwrap().as_str() {
            "remove_relationship_record" => RemovalEffect::RemoveRelationshipRecord,
            "remove_semantic_relationship_index" => RemovalEffect::RemoveSemanticRelationshipIndex,
            "create_idempotency_tombstone" => RemovalEffect::CreateIdempotencyTombstone,
            _ => panic!("prepared effect"),
        });
    }
    let preview = pmc_domain::execution::RemoveRelationshipPreview::from_persistence(
        PreparedIntentId::parse(id).unwrap(),
        RelationshipId::parse(rid).unwrap(),
        ver(rv),
        es,
        rel_kind(&k),
        rel_purpose(Some(&p)),
        effects,
        cls(&cl),
        RemovalPolicyDecision::Allowed,
        ev,
        ts(ex),
        CancellationPolicy::NotCancellableAfterSubmit,
        ch,
    );
    PreparedIntent::from_persistence(preview, PayloadDigest::parse(d).unwrap())
}

fn decode_audits(c: &Connection) -> Vec<AuditEvent> {
    let mut q = c
        .prepare("SELECT a.id,a.occurred_at,a.actor,a.module,a.event_code,a.target_type,a.target_id,a.correlation_id,a.policy_outcome,a.approval_outcome,a.execution_outcome,a.effect_scope FROM relationship_replay_operations ro JOIN relationship_replay_audits ra ON ra.idempotency_id=ro.idempotency_id JOIN audit_events a ON a.id=ra.audit_event_id ORDER BY ro.operation_ordinal,ra.ordinal")
        .unwrap();
    let decoded = q
        .query_map([], |r| {
            let id: String = r.get(0)?;
            let target_type: String = r.get(5)?;
            let target_id: String = r.get(6)?;
            let target = match target_type.as_str() {
                "portfolio" => AuditTarget::Portfolio(PortfolioId::parse(target_id).unwrap()),
                "product" => AuditTarget::Product(ProductId::parse(target_id).unwrap()),
                "initiative" => AuditTarget::Initiative(InitiativeId::parse(target_id).unwrap()),
                "roadmap" => AuditTarget::Roadmap(RoadmapId::parse(target_id).unwrap()),
                "kpi_definition" => AuditTarget::Kpi(KpiId::parse(target_id).unwrap()),
                "project" => AuditTarget::Project(ProjectId::parse(target_id).unwrap()),
                "milestone" => AuditTarget::Milestone(MilestoneId::parse(target_id).unwrap()),
                "stakeholder" => AuditTarget::Stakeholder(StakeholderId::parse(target_id).unwrap()),
                "relationship" => {
                    AuditTarget::Relationship(RelationshipId::parse(target_id).unwrap())
                }
                _ => panic!("audit target type"),
            };
            let mut effects = Vec::new();
            let mut e = c
            .prepare(
                "SELECT effect_code FROM audit_effects WHERE audit_event_id=?1 ORDER BY ordinal",
            )
            .unwrap();
            for value in e
                .query_map([id.as_str()], |row| row.get::<_, String>(0))
                .unwrap()
            {
                effects.push(AuditEffectCode::parse(value.unwrap()).unwrap());
            }
            Ok(AuditEvent::new(
                AuditEventId::parse(id).unwrap(),
                ts(r.get(1)?),
                AuditActor::from_persisted(r.get::<_, String>(2)?.as_str()).unwrap(),
                AuditAction::new(
                    AuditModule::from_persisted(r.get::<_, String>(3)?.as_str()).unwrap(),
                    AuditEventCode::parse(r.get::<_, String>(4)?).unwrap(),
                    target,
                ),
                CorrelationId::parse(r.get::<_, String>(7)?).unwrap(),
                AuditDisposition::new(
                    AuditPolicyOutcome::from_persisted(r.get::<_, String>(8)?.as_str()).unwrap(),
                    AuditApprovalOutcome::from_persisted(r.get::<_, String>(9)?.as_str()).unwrap(),
                    AuditExecutionOutcome::from_persisted(r.get::<_, String>(10)?.as_str())
                        .unwrap(),
                    AuditEffectScope::from_persisted(r.get::<_, String>(11)?.as_str()).unwrap(),
                    effects,
                )
                .unwrap(),
            ))
        })
        .unwrap()
        .map(|v| v.unwrap())
        .collect::<Vec<_>>();
    let stored_count: i64 = c
        .query_row("SELECT count(*) FROM audit_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        decoded.len() as i64,
        stored_count,
        "every audit must be linked exactly once"
    );
    decoded
}

fn decode_h2b(c: &Connection) -> Vec<pmc_domain::relationships::RelationshipH2bReplayCapsule> {
    let mut q = c.prepare("SELECT o.idempotency_id,o.correlation_id,o.operation_ordinal,o.operation,o.result_kind,o.result_reference,o.error_code,o.error_message_key,o.error_retryable,h.relationship_id,h.prepared_intent_id,h.actor,h.acknowledged_payload_digest,h.confirmation_digest,h.result_prepared_intent_id,h.result_relationship_id FROM relationship_replay_operations o JOIN relationship_h2b_command_results h ON h.idempotency_id=o.idempotency_id ORDER BY o.operation_ordinal").unwrap();
    q.query_map([], |row| {
        let id: String = row.get(0)?;
        let correlation: String = row.get(1)?;
        let ordinal: i64 = row.get(2)?;
        let operation: String = row.get(3)?;
        let result_kind: String = row.get(4)?;
        let error_code_value: Option<String> = row.get(6)?;
        let error_message_key: Option<String> = row.get(7)?;
        let error_retryable: Option<i64> = row.get(8)?;
        let relationship_id: Option<String> = row.get(9)?;
        let prepared_id: Option<String> = row.get(10)?;
        let actor: Option<String> = row.get(11)?;
        let acknowledged: Option<String> = row.get(12)?;
        let confirmation: Option<String> = row.get(13)?;
        let result_prepared: Option<String> = row.get(14)?;
        let result_relationship: Option<String> = row.get(15)?;
        let audits = replay_audits(c, &id);
        let command = match operation.as_str() {
            "prepare_remove" => RelationshipH2bPersistenceCommand::Prepare {
                relationship_id: RelationshipId::parse(relationship_id.unwrap()).unwrap(),
            },
            "cancel_remove" => RelationshipH2bPersistenceCommand::Cancel {
                prepared_id: PreparedIntentId::parse(prepared_id.unwrap()).unwrap(),
                actor: AuditActor::from_persisted(actor.unwrap().as_str()).unwrap(),
            },
            "execute_remove" => RelationshipH2bPersistenceCommand::Execute {
                prepared_id: PreparedIntentId::parse(prepared_id.unwrap()).unwrap(),
                acknowledged_payload_digest: PayloadDigest::parse(acknowledged.unwrap()).unwrap(),
                confirmation_digest: PayloadDigest::parse(confirmation.unwrap()).unwrap(),
                actor: AuditActor::from_persisted(actor.unwrap().as_str()).unwrap(),
            },
            _ => panic!("h2b operation"),
        };
        let result = match result_kind.as_str() {
            "prepared" => RelationshipH2bPersistenceResult::Prepared(decode_prepared(
                c,
                &result_prepared.unwrap(),
            )),
            "cancelled" => RelationshipH2bPersistenceResult::Cancelled,
            "removal" => {
                RelationshipH2bPersistenceResult::Removal(pmc_domain::execution::RemovalOutcome {
                    relationship_id: RelationshipId::parse(result_relationship.unwrap()).unwrap(),
                    audit_event_ids: audits.clone(),
                })
            }
            "rejection" => RelationshipH2bPersistenceResult::Rejection(DomainError::new(
                error_code(error_code_value.unwrap().as_str()),
                MessageKey::parse(error_message_key.unwrap()).unwrap(),
                CorrelationId::parse(correlation.clone()).unwrap(),
                error_retryable.unwrap() != 0,
            )),
            _ => panic!("h2b result"),
        };
        Ok(
            pmc_domain::relationships::RelationshipH2bReplayCapsule::new(
                IdempotencyId::parse(id).unwrap(),
                CorrelationId::parse(correlation).unwrap(),
                ordinal as u64,
                command,
                result,
                audits,
            ),
        )
    })
    .unwrap()
    .map(|v| v.unwrap())
    .collect()
}

fn replay_audits(c: &Connection, id: &str) -> Vec<AuditEventId> {
    let mut q = c
        .prepare("SELECT audit_event_id FROM relationship_replay_audits WHERE idempotency_id=?1 ORDER BY ordinal")
        .unwrap();
    q.query_map([id], |r| r.get::<_, String>(0))
        .unwrap()
        .map(|v| AuditEventId::parse(v.unwrap()).unwrap())
        .collect()
}
fn completed_ids(c: &Connection) -> Vec<PreparedIntentId> {
    let mut q = c.prepare("SELECT prepared_intent_id FROM relationship_h2b_command_results WHERE result_kind='removal' ORDER BY idempotency_id").unwrap();
    q.query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(|v| PreparedIntentId::parse(v.unwrap()).unwrap())
        .collect()
}
fn tombstones(c: &Connection, kind: &str) -> Vec<IdempotencyId> {
    let mut q = c.prepare("SELECT idempotency_id FROM relationship_replay_tombstones WHERE kind=?1 ORDER BY idempotency_id").unwrap();
    q.query_map([kind], |r| r.get::<_, String>(0))
        .unwrap()
        .map(|v| IdempotencyId::parse(v.unwrap()).unwrap())
        .collect()
}

fn decode(c: &Connection) -> RelationshipH2bPersistenceSnapshot {
    let (ordinary_id, ordinary_correlation, ordinary_ordinal, ordinary_reference):
        (String, String, i64, String) = c
        .query_row(
            "SELECT idempotency_id,correlation_id,operation_ordinal,result_reference FROM relationship_replay_operations WHERE operation='link'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    let (ordinary_kind, ordinary_purpose, ordinary_version, ordinary_class, ordinary_created, ordinary_updated, ordinary_scope):
        (String, Option<String>, i64, String, i64, i64, String) = c
        .query_row(
            "SELECT relationship_kind,purpose,result_version,result_classification,result_created_at,result_updated_at,effect_scope FROM relationship_link_command_results WHERE idempotency_id=?1",
            [&ordinary_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
        )
        .unwrap();
    let mut command_q = c.prepare("SELECT endpoint_type,endpoint_id,expected_version,snapshot_version,classification,parent_project_id FROM relationship_link_command_endpoints WHERE idempotency_id=?1 ORDER BY ordinal").unwrap();
    let mut expected_versions = Vec::new();
    let command_endpoints = command_q
        .query_map([ordinary_id.as_str()], |row| {
            expected_versions.push(ver(row.get::<_, i64>(2)?));
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .unwrap()
        .map(|v| {
            let (t, i, v, cl, parent) = v.unwrap();
            match t.as_str() {
                "portfolio" => EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
                    PortfolioId::parse(i).unwrap(),
                    ver(v),
                    cls(&cl),
                )),
                "product" => EndpointSnapshot::Product(ProductSnapshot::new(
                    ProductId::parse(i).unwrap(),
                    ver(v),
                    cls(&cl),
                )),
                "milestone" => {
                    EndpointSnapshot::Milestone(pmc_domain::relationships::MilestoneSnapshot::new(
                        MilestoneId::parse(i).unwrap(),
                        ProjectId::parse(parent.unwrap()).unwrap(),
                        ver(v),
                        cls(&cl),
                    ))
                }
                _ => panic!("command endpoint"),
            }
        })
        .collect::<Vec<_>>();
    let mut result_q = c.prepare("SELECT endpoint_type,endpoint_id,endpoint_version,classification,parent_project_id FROM relationship_link_result_endpoints WHERE idempotency_id=?1 ORDER BY ordinal").unwrap();
    let result_endpoints = result_q
        .query_map([ordinary_id.as_str()], ep_row)
        .unwrap()
        .map(|v| v.unwrap())
        .collect::<Vec<_>>();
    let result_record = RelationshipRecord::rehydrate(RelationshipPersistenceRecord {
        id: RelationshipId::parse(ordinary_reference).unwrap(),
        kind: rel_kind(&ordinary_kind),
        endpoints: result_endpoints,
        purpose: rel_purpose(ordinary_purpose.as_deref()),
        classification: cls(&ordinary_class),
        version: ver(ordinary_version),
        created_at: ts(ordinary_created),
        updated_at: ts(ordinary_updated),
    })
    .unwrap();
    let historical_record = result_record.clone();
    let ordinary = RelationshipReplayCapsule::new(
        IdempotencyId::parse(ordinary_id.clone()).unwrap(),
        CorrelationId::parse(ordinary_correlation).unwrap(),
        ordinary_ordinal as u64,
        RelationshipPersistenceCommand::Link {
            id: result_record.id().clone(),
            kind: rel_kind(&ordinary_kind),
            endpoints: command_endpoints,
            expected_versions,
            purpose: rel_purpose(ordinary_purpose.as_deref()),
        },
        RelationshipPersistenceResult::Relationship(
            pmc_domain::relationships::MutationOutcome::from_persistence(
                result_record,
                replay_audits(c, &ordinary_id),
                match ordinary_scope.as_str() {
                    "complete" => AuditEffectScope::Complete,
                    "none" => AuditEffectScope::None,
                    "partial" => AuditEffectScope::Partial,
                    _ => panic!("effect scope"),
                },
            ),
        ),
        replay_audits(c, &ordinary_id),
    );
    let has_current_relationship: bool = c
        .query_row(
            "SELECT 1 FROM relationships WHERE id='relationship-sqlite-h2b'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .unwrap()
        .is_some();
    let r = historical_record;
    let h2b = decode_h2b(c);
    let no_longer_pending = h2b
        .iter()
        .filter_map(|capsule| match (capsule.command(), capsule.result()) {
            (
                RelationshipH2bPersistenceCommand::Cancel { prepared_id, .. },
                RelationshipH2bPersistenceResult::Cancelled,
            )
            | (
                RelationshipH2bPersistenceCommand::Execute { prepared_id, .. },
                RelationshipH2bPersistenceResult::Removal(_),
            ) => Some(prepared_id),
            _ => None,
        })
        .collect::<std::collections::HashSet<_>>();
    let pending = h2b
        .iter()
        .filter_map(|capsule| match capsule.result() {
            RelationshipH2bPersistenceResult::Prepared(p)
                if !no_longer_pending.contains(p.id()) =>
            {
                Some(p.clone())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    RelationshipH2bPersistenceSnapshot::from_persistence(
        vec![],
        if has_current_relationship {
            vec![r.clone()]
        } else {
            vec![]
        },
        vec![r],
        vec![ordinary],
        h2b,
        decode_audits(c),
        pending,
        completed_ids(c),
        tombstones(c, "ordinary"),
        tombstones(c, "h2b"),
    )
    .unwrap()
}

fn decode_provenance(kind: &str, reference: Option<String>) -> pmc_domain::provenance::Provenance {
    match kind {
        "user_entered" => pmc_domain::provenance::Provenance::UserEntered,
        "authoritative_transition" => pmc_domain::provenance::Provenance::AuthoritativeTransition(
            pmc_domain::provenance::ProvenanceReference::parse(reference.unwrap()).unwrap(),
        ),
        "synthetic_fixture" => pmc_domain::provenance::Provenance::SyntheticFixture(
            pmc_domain::provenance::ProvenanceReference::parse(reference.unwrap()).unwrap(),
        ),
        _ => panic!("provenance"),
    }
}
fn decode_stakeholders(c: &Connection) -> Vec<StakeholderRecord> {
    let mut q=c.prepare("SELECT a.id,s.name,s.kind,a.classification,s.provenance_kind,s.provenance_reference,a.version,a.created_at,a.updated_at FROM aggregate_registry a JOIN stakeholders s ON s.id=a.id WHERE a.aggregate_type='stakeholder' ORDER BY a.id").unwrap();
    q.query_map([], |r| {
        let id: String = r.get(0)?;
        Ok(StakeholderRecord::rehydrate(StakeholderPersistenceRecord {
            id: StakeholderId::parse(id).unwrap(),
            name: pmc_domain::relationships::StakeholderName::parse(r.get::<_, String>(1)?)
                .unwrap(),
            kind: StakeholderKind::from_persisted(r.get::<_, String>(2)?.as_str()).unwrap(),
            classification: cls(&r.get::<_, String>(3)?),
            provenance: decode_provenance(r.get::<_, String>(4)?.as_str(), r.get(5)?),
            version: ver(r.get(6)?),
            created_at: ts(r.get(7)?),
            updated_at: ts(r.get(8)?),
        })
        .unwrap())
    })
    .unwrap()
    .map(|v| v.unwrap())
    .collect()
}
fn decode_current_relationships(c: &Connection) -> Vec<RelationshipRecord> {
    let mut ids = c
        .prepare("SELECT id FROM relationships ORDER BY id")
        .unwrap();
    ids.query_map([],|r|r.get::<_,String>(0)).unwrap().map(|v|{let id=v.unwrap();let(k,p,v,cl,ca,ua):(String,Option<String>,i64,String,i64,i64)=c.query_row("SELECT r.kind,r.purpose,a.version,a.classification,a.created_at,a.updated_at FROM relationships r JOIN aggregate_registry a ON a.id=r.id AND a.aggregate_type='relationship' WHERE r.id=?1",[id.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap();let mut ep=c.prepare("SELECT target_type,target_id,target_version,target_classification,parent_project_id FROM relationship_endpoints WHERE relationship_id=?1 ORDER BY ordinal").unwrap();let endpoints=ep.query_map([id.as_str()],ep_row).unwrap().map(|v|v.unwrap()).collect();RelationshipRecord::rehydrate(RelationshipPersistenceRecord{id:RelationshipId::parse(id).unwrap(),kind:rel_kind(&k),endpoints,purpose:rel_purpose(p.as_deref()),classification:cls(&cl),version:ver(v),created_at:ts(ca),updated_at:ts(ua)}).unwrap()}).collect()
}
fn decode_link_capsule(
    c: &Connection,
    id: &str,
    correlation: String,
    ordinal: i64,
    result_ref: String,
) -> RelationshipReplayCapsule {
    let(k,p,v,cl,ca,ua,scope):(String,Option<String>,i64,String,i64,i64,String)=c.query_row("SELECT relationship_kind,purpose,result_version,result_classification,result_created_at,result_updated_at,effect_scope FROM relationship_link_command_results WHERE idempotency_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).unwrap();
    let mut q=c.prepare("SELECT endpoint_type,endpoint_id,expected_version,snapshot_version,classification,parent_project_id FROM relationship_link_command_endpoints WHERE idempotency_id=?1 ORDER BY ordinal").unwrap();
    let mut expected = Vec::new();
    let command_eps = q
        .query_map([id], |r| {
            expected.push(ver(r.get::<_, i64>(2)?));
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })
        .unwrap()
        .map(|v| {
            let (t, i, v, cl, parent) = v.unwrap();
            endpoint_from_parts(&t, &i, v, &cl, parent)
        })
        .collect::<Vec<_>>();
    let mut rq=c.prepare("SELECT endpoint_type,endpoint_id,endpoint_version,classification,parent_project_id FROM relationship_link_result_endpoints WHERE idempotency_id=?1 ORDER BY ordinal").unwrap();
    let result_eps = rq
        .query_map([id], ep_row)
        .unwrap()
        .map(|v| v.unwrap())
        .collect::<Vec<_>>();
    let result = RelationshipRecord::rehydrate(RelationshipPersistenceRecord {
        id: RelationshipId::parse(result_ref).unwrap(),
        kind: rel_kind(&k),
        endpoints: result_eps,
        purpose: rel_purpose(p.as_deref()),
        classification: cls(&cl),
        version: ver(v),
        created_at: ts(ca),
        updated_at: ts(ua),
    })
    .unwrap();
    let audits = replay_audits(c, id);
    RelationshipReplayCapsule::new(
        IdempotencyId::parse(id).unwrap(),
        CorrelationId::parse(correlation).unwrap(),
        ordinal as u64,
        RelationshipPersistenceCommand::Link {
            id: result.id().clone(),
            kind: rel_kind(&k),
            endpoints: command_eps,
            expected_versions: expected,
            purpose: rel_purpose(p.as_deref()),
        },
        RelationshipPersistenceResult::Relationship(
            pmc_domain::relationships::MutationOutcome::from_persistence(
                result,
                audits.clone(),
                match scope.as_str() {
                    "complete" => AuditEffectScope::Complete,
                    "none" => AuditEffectScope::None,
                    "partial" => AuditEffectScope::Partial,
                    _ => panic!("scope"),
                },
            ),
        ),
        audits,
    )
}
fn endpoint_from_parts(
    t: &str,
    id: &str,
    v: i64,
    cl: &str,
    parent: Option<String>,
) -> EndpointSnapshot {
    match t {
        "portfolio" => EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
            PortfolioId::parse(id).unwrap(),
            ver(v),
            cls(cl),
        )),
        "product" => EndpointSnapshot::Product(ProductSnapshot::new(
            ProductId::parse(id).unwrap(),
            ver(v),
            cls(cl),
        )),
        "initiative" => {
            EndpointSnapshot::Initiative(pmc_domain::relationships::InitiativeSnapshot::new(
                InitiativeId::parse(id).unwrap(),
                ver(v),
                cls(cl),
            ))
        }
        "roadmap" => EndpointSnapshot::Roadmap(pmc_domain::relationships::RoadmapSnapshot::new(
            RoadmapId::parse(id).unwrap(),
            ver(v),
            cls(cl),
        )),
        "kpi" => EndpointSnapshot::Kpi(pmc_domain::relationships::KpiSnapshot::new(
            KpiId::parse(id).unwrap(),
            ver(v),
            cls(cl),
        )),
        "project" => EndpointSnapshot::Project(pmc_domain::relationships::ProjectSnapshot::new(
            ProjectId::parse(id).unwrap(),
            ver(v),
            cls(cl),
        )),
        "milestone" => {
            EndpointSnapshot::Milestone(pmc_domain::relationships::MilestoneSnapshot::new(
                MilestoneId::parse(id).unwrap(),
                ProjectId::parse(parent.unwrap()).unwrap(),
                ver(v),
                cls(cl),
            ))
        }
        "stakeholder" => {
            EndpointSnapshot::Stakeholder(pmc_domain::relationships::StakeholderSnapshot::new(
                StakeholderId::parse(id).unwrap(),
                ver(v),
                cls(cl),
            ))
        }
        _ => panic!("endpoint"),
    }
}
type StakeholderCommandRow = (
    String,
    String,
    Option<i64>,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
    String,
    String,
    String,
    Option<String>,
    i64,
    i64,
    i64,
    String,
);

fn decode_ordinary(c: &Connection) -> RelationshipPersistenceSnapshot {
    let stakeholders = decode_stakeholders(c);
    let relationships = decode_current_relationships(c);
    let mut oq=c.prepare("SELECT idempotency_id,correlation_id,operation_ordinal,operation,result_reference FROM relationship_replay_operations WHERE operation IN ('create_stakeholder','update_stakeholder','link') ORDER BY operation_ordinal").unwrap();
    let rows = oq
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })
        .unwrap()
        .map(|v| v.unwrap())
        .collect::<Vec<_>>();
    let mut replay = Vec::new();
    for (id, correlation, ordinal, operation, result_ref) in rows {
        let audits = replay_audits(c, &id);
        match operation.as_str() {
            "link" => replay.push(decode_link_capsule(
                c,
                &id,
                correlation,
                ordinal,
                result_ref,
            )),
            "create_stakeholder" | "update_stakeholder" => {
                let(kind,cmd_id,expected,name,cmd_class,cmd_stakeholder_kind,cmd_pk,cmd_pref,result_name,result_kind,result_class,result_pk,result_pref,result_ver,result_created,result_updated,scope): StakeholderCommandRow = c.query_row("SELECT command_kind,command_id,command_expected_version,command_name,command_classification,command_stakeholder_kind,command_provenance_kind,command_provenance_reference,result_name,result_stakeholder_kind,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at,effect_scope FROM relationship_stakeholder_command_results WHERE idempotency_id=?1",[id.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?,r.get(11)?,r.get(12)?,r.get(13)?,r.get(14)?,r.get(15)?,r.get(16)?))).unwrap();
                let result = StakeholderRecord::rehydrate(StakeholderPersistenceRecord {
                    id: StakeholderId::parse(result_ref).unwrap(),
                    name: pmc_domain::relationships::StakeholderName::parse(result_name).unwrap(),
                    kind: StakeholderKind::from_persisted(&result_kind).unwrap(),
                    classification: cls(&result_class),
                    provenance: decode_provenance(&result_pk, result_pref),
                    version: ver(result_ver),
                    created_at: ts(result_created),
                    updated_at: ts(result_updated),
                })
                .unwrap();
                let command = if kind == "create_stakeholder" {
                    RelationshipPersistenceCommand::CreateStakeholder {
                        id: StakeholderId::parse(cmd_id).unwrap(),
                        name: pmc_domain::relationships::StakeholderName::parse(name).unwrap(),
                        kind: StakeholderKind::from_persisted(
                            cmd_stakeholder_kind.as_deref().unwrap(),
                        )
                        .unwrap(),
                        classification: cmd_class.as_deref().map(cls),
                        provenance: decode_provenance(cmd_pk.as_deref().unwrap(), cmd_pref),
                    }
                } else {
                    RelationshipPersistenceCommand::UpdateStakeholder {
                        id: StakeholderId::parse(cmd_id).unwrap(),
                        expected_version: ver(expected.unwrap()),
                        name: pmc_domain::relationships::StakeholderName::parse(name).unwrap(),
                        classification: cmd_class.as_deref().map(cls),
                    }
                };
                let outcome = pmc_domain::relationships::MutationOutcome::from_persistence(
                    result,
                    audits.clone(),
                    match scope.as_str() {
                        "complete" => AuditEffectScope::Complete,
                        "none" => AuditEffectScope::None,
                        "partial" => AuditEffectScope::Partial,
                        _ => panic!("scope"),
                    },
                );
                replay.push(RelationshipReplayCapsule::new(
                    IdempotencyId::parse(id).unwrap(),
                    CorrelationId::parse(correlation).unwrap(),
                    ordinal as u64,
                    command,
                    RelationshipPersistenceResult::Stakeholder(outcome),
                    audits,
                ));
            }
            _ => panic!("ordinary operation"),
        }
    }
    let decoded_audits = decode_audits(c);
    let mut audit_order = c
        .prepare("SELECT ra.audit_event_id FROM relationship_replay_audits ra JOIN relationship_replay_operations ro ON ro.idempotency_id=ra.idempotency_id ORDER BY ro.operation_ordinal, ra.ordinal")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|row| AuditEventId::parse(row.unwrap()).unwrap())
        .collect::<Vec<_>>();
    let audits = audit_order
        .drain(..)
        .map(|id| {
            decoded_audits
                .iter()
                .find(|audit| audit.id() == &id)
                .unwrap()
                .clone()
        })
        .collect::<Vec<_>>();
    RelationshipPersistenceSnapshot::validate(stakeholders, relationships, replay, audits)
        .unwrap_or_else(|_| panic!("ordinary persistence validation failed"))
}

#[test]
fn sqlite_relationship_h2b_round_trip_preserves_typed_capsules_and_pending_state() {
    let s = fixture();
    let expected = s.persistence_snapshot_with_h2b().unwrap();
    let db = TempLedger::new();
    SqliteProductLedger::open(&db.0).unwrap();
    let mut c = open_test_connection(&db.0);
    persist(&mut c, &expected);
    let decoded = decode(&c);
    assert_eq!(decoded, expected);
    let mut restored = InMemoryRelationshipService::rehydrate_with_h2b(
        FixedClock,
        InMemoryEndpointCatalog::new([
            EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
                PortfolioId::parse("portfolio-sqlite-h2b").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Public,
            )),
            EndpointSnapshot::Product(ProductSnapshot::new(
                ProductId::parse("product-sqlite-h2b").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Internal,
            )),
        ]),
        AuditIds::default(),
        ExecIds(500),
        AllowRemoval,
        AllowApproval,
        decoded,
    )
    .unwrap();
    let before = restored.audit_events().len();
    let replay = restored
        .link_portfolio_product(LinkPortfolioProduct {
            id: RelationshipId::parse("relationship-sqlite-h2b").unwrap(),
            portfolio_id: PortfolioId::parse("portfolio-sqlite-h2b").unwrap(),
            product_id: ProductId::parse("product-sqlite-h2b").unwrap(),
            expected_portfolio_version: AggregateVersion::initial(),
            expected_product_version: AggregateVersion::initial(),
            context: pmc_domain::relationships::OperationContext {
                idempotency_id: IdempotencyId::parse("sqlite-rel-link").unwrap(),
                correlation_id: CorrelationId::parse("changed-correlation").unwrap(),
            },
        })
        .unwrap();
    assert_eq!(replay.value().id().as_str(), "relationship-sqlite-h2b");
    assert_eq!(restored.audit_events().len(), before);
    assert_eq!(restored.relationship_count(), 1);
    assert_eq!(
        restored
            .persistence_snapshot_with_h2b()
            .unwrap()
            .pending()
            .len(),
        1
    );
}

#[test]
fn sqlite_relationship_h2b_round_trip_preserves_pre_submit_cancellation() {
    let mut original = linked_fixture();
    let relationship_id = RelationshipId::parse("relationship-sqlite-h2b").unwrap();
    let prepared = original
        .prepare_remove_relationship(
            PrepareRemoveRelationship {
                relationship_id: relationship_id.clone(),
                context: ctx("prepare-cancel"),
            },
            &Recovery(
                RecoveryEvidence::new(
                    RecoveryEvidenceId::parse("recovery-sqlite-h2b").unwrap(),
                    "synthetic recovery",
                    ts(900),
                    relationship_id.clone(),
                    true,
                )
                .unwrap(),
            ),
        )
        .unwrap();
    original
        .cancel_remove_relationship(
            prepared.id(),
            AuditActor::HeadOfProducts,
            ctx("cancel-before-submit"),
        )
        .unwrap();
    let expected = original.persistence_snapshot_with_h2b().unwrap();
    let db = TempLedger::new();
    SqliteProductLedger::open(&db.0).unwrap();
    let mut c = open_test_connection(&db.0);
    persist(&mut c, &expected);
    let decoded = decode(&c);
    assert_eq!(decoded, expected);
    let mut restored = InMemoryRelationshipService::rehydrate_with_h2b(
        FixedClock,
        InMemoryEndpointCatalog::new([
            EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
                PortfolioId::parse("portfolio-sqlite-h2b").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Public,
            )),
            EndpointSnapshot::Product(ProductSnapshot::new(
                ProductId::parse("product-sqlite-h2b").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Internal,
            )),
        ]),
        AuditIds::default(),
        ExecIds(500),
        AllowRemoval,
        AllowApproval,
        decoded,
    )
    .unwrap();
    let audits = restored.audit_events().len();
    restored
        .cancel_remove_relationship(
            prepared.id(),
            AuditActor::HeadOfProducts,
            ctx("cancel-before-submit"),
        )
        .unwrap();
    assert_eq!(restored.audit_events().len(), audits);
    assert_eq!(restored.relationship_count(), 1);
}

#[test]
fn sqlite_relationship_h2b_round_trip_preserves_audited_nonretryable_rejection() {
    let mut original = linked_fixture();
    let relationship_id = RelationshipId::parse("relationship-sqlite-h2b").unwrap();
    let prepared = original
        .prepare_remove_relationship(
            PrepareRemoveRelationship {
                relationship_id: relationship_id.clone(),
                context: ctx("prepare-reject"),
            },
            &Recovery(
                RecoveryEvidence::new(
                    RecoveryEvidenceId::parse("recovery-sqlite-h2b").unwrap(),
                    "synthetic recovery",
                    ts(900),
                    relationship_id.clone(),
                    true,
                )
                .unwrap(),
            ),
        )
        .unwrap();
    let mut request = execute(&prepared, "execute-reject", "REMOVE wrong-challenge");
    let expected_error = original
        .approve_and_execute_remove_relationship(
            request.clone(),
            &Recovery(
                RecoveryEvidence::new(
                    RecoveryEvidenceId::parse("recovery-sqlite-h2b").unwrap(),
                    "synthetic recovery",
                    ts(900),
                    relationship_id.clone(),
                    true,
                )
                .unwrap(),
            ),
        )
        .expect_err("rejection");
    assert_eq!(expected_error.code(), ErrorCode::SecurityPolicyDenied);
    assert!(!expected_error.retryable());
    let expected = original.persistence_snapshot_with_h2b().unwrap();
    let db = TempLedger::new();
    SqliteProductLedger::open(&db.0).unwrap();
    let mut c = open_test_connection(&db.0);
    persist(&mut c, &expected);
    let decoded = decode(&c);
    assert_eq!(decoded, expected);
    let mut restored = InMemoryRelationshipService::rehydrate_with_h2b(
        FixedClock,
        InMemoryEndpointCatalog::new([
            EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
                PortfolioId::parse("portfolio-sqlite-h2b").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Public,
            )),
            EndpointSnapshot::Product(ProductSnapshot::new(
                ProductId::parse("product-sqlite-h2b").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Internal,
            )),
        ]),
        AuditIds::default(),
        ExecIds(500),
        AllowRemoval,
        AllowApproval,
        decoded,
    )
    .unwrap();
    let audits = restored.audit_events().len();
    let replay_error = restored
        .approve_and_execute_remove_relationship(
            request.clone(),
            &Recovery(
                RecoveryEvidence::new(
                    RecoveryEvidenceId::parse("recovery-sqlite-h2b").unwrap(),
                    "synthetic recovery",
                    ts(900),
                    relationship_id.clone(),
                    true,
                )
                .unwrap(),
            ),
        )
        .expect_err("replay rejection");
    assert_eq!(replay_error, expected_error);
    assert_eq!(restored.audit_events().len(), audits);
    request.confirmation = "REMOVE another-challenge".to_owned();
    assert_eq!(
        restored
            .approve_and_execute_remove_relationship(
                request,
                &Recovery(
                    RecoveryEvidence::new(
                        RecoveryEvidenceId::parse("recovery-sqlite-h2b").unwrap(),
                        "synthetic recovery",
                        ts(900),
                        relationship_id,
                        true
                    )
                    .unwrap()
                )
            )
            .unwrap_err()
            .code(),
        ErrorCode::DomainIdempotencyConflict
    );
}

#[test]
fn sqlite_relationship_h2b_round_trip_preserves_terminal_removal_without_resurrection() {
    let mut original = linked_fixture();
    let relationship_id = RelationshipId::parse("relationship-sqlite-h2b").unwrap();
    let prepared = original
        .prepare_remove_relationship(
            PrepareRemoveRelationship {
                relationship_id: relationship_id.clone(),
                context: ctx("prepare-terminal"),
            },
            &Recovery(
                RecoveryEvidence::new(
                    RecoveryEvidenceId::parse("recovery-sqlite-h2b").unwrap(),
                    "synthetic recovery",
                    ts(900),
                    relationship_id.clone(),
                    true,
                )
                .unwrap(),
            ),
        )
        .unwrap();
    let outcome = original
        .approve_and_execute_remove_relationship(
            execute(
                &prepared,
                "execute-terminal",
                prepared.confirmation_challenge(),
            ),
            &Recovery(
                RecoveryEvidence::new(
                    RecoveryEvidenceId::parse("recovery-sqlite-h2b").unwrap(),
                    "synthetic recovery",
                    ts(900),
                    relationship_id.clone(),
                    true,
                )
                .unwrap(),
            ),
        )
        .unwrap();
    let expected = original.persistence_snapshot_with_h2b().unwrap();
    assert!(expected.relationships().is_empty());
    assert_eq!(expected.completed().len(), 1);
    assert_eq!(expected.tombstoned_ordinary().len(), 1);
    assert_eq!(expected.tombstoned_h2b().len(), 1);
    let db = TempLedger::new();
    SqliteProductLedger::open(&db.0).unwrap();
    let mut c = open_test_connection(&db.0);
    persist(&mut c, &expected);
    let decoded = decode(&c);
    assert_eq!(decoded, expected);
    let mut restored = InMemoryRelationshipService::rehydrate_with_h2b(
        FixedClock,
        InMemoryEndpointCatalog::new([
            EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
                PortfolioId::parse("portfolio-sqlite-h2b").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Public,
            )),
            EndpointSnapshot::Product(ProductSnapshot::new(
                ProductId::parse("product-sqlite-h2b").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Internal,
            )),
        ]),
        AuditIds::default(),
        ExecIds(500),
        AllowRemoval,
        AllowApproval,
        decoded,
    )
    .unwrap();
    assert_eq!(restored.relationship_count(), 0);
    let audits = restored.audit_events().len();
    assert_eq!(
        restored
            .approve_and_execute_remove_relationship(
                execute(
                    &prepared,
                    "execute-terminal",
                    prepared.confirmation_challenge()
                ),
                &Recovery(
                    RecoveryEvidence::new(
                        RecoveryEvidenceId::parse("recovery-sqlite-h2b").unwrap(),
                        "synthetic recovery",
                        ts(900),
                        relationship_id.clone(),
                        true
                    )
                    .unwrap()
                )
            )
            .unwrap(),
        outcome
    );
    assert_eq!(restored.audit_events().len(), audits);
    assert_eq!(
        restored
            .link_portfolio_product(LinkPortfolioProduct {
                id: relationship_id,
                portfolio_id: PortfolioId::parse("portfolio-sqlite-h2b").unwrap(),
                product_id: ProductId::parse("product-sqlite-h2b").unwrap(),
                expected_portfolio_version: AggregateVersion::initial(),
                expected_product_version: AggregateVersion::initial(),
                context: ctx("reuse-removed")
            })
            .unwrap_err()
            .code(),
        ErrorCode::DomainConflict
    );
}

#[test]
fn sqlite_relationship_ordinary_full_contract_round_trips_all_link_families_and_stakeholder_fanout()
{
    let original = ordinary_full_fixture();
    let expected = original.persistence_snapshot().unwrap();
    assert_eq!(
        expected
            .replay()
            .iter()
            .filter(|x| matches!(x.command(), RelationshipPersistenceCommand::Link { .. }))
            .count(),
        9
    );
    assert!(expected.replay().iter().any(|x| matches!(
        x.command(),
        RelationshipPersistenceCommand::CreateStakeholder { .. }
    )));
    assert!(expected.replay().iter().any(|x| matches!(
        x.command(),
        RelationshipPersistenceCommand::UpdateStakeholder { .. }
    )));
    for relationship_id in [
        "ordinary-rel-stakeholder-responsibility",
        "ordinary-rel-stakeholder-dependency",
        "ordinary-rel-stakeholder-milestone",
    ] {
        let relationship = expected
            .relationships()
            .iter()
            .find(|value| value.id().as_str() == relationship_id)
            .unwrap();
        assert_eq!(
            relationship.classification(),
            DataClassification::Restricted
        );
        assert_eq!(relationship.version(), AggregateVersion::new(2).unwrap());
    }
    let db = TempLedger::new();
    SqliteProductLedger::open(&db.0).unwrap();
    let mut connection = open_test_connection(&db.0);
    persist_ordinary(&mut connection, &expected);
    for table in [
        "relationship_link_command_endpoints",
        "relationship_link_result_endpoints",
    ] {
        let parent: String = connection
            .query_row(
                &format!(
                    "SELECT parent_project_id FROM {table} WHERE idempotency_id='sqlite-rel-ordinary-link-stakeholder-milestone' AND endpoint_type='milestone'"
                ),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(parent, "ordinary-project");
    }
    let decoded = decode_ordinary(&connection);
    assert_eq!(decoded, expected);
    let mut restored = InMemoryRelationshipService::rehydrate(
        FixedClock,
        InMemoryEndpointCatalog::new([
            EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
                PortfolioId::parse("ordinary-portfolio").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Public,
            )),
            EndpointSnapshot::Product(ProductSnapshot::new(
                ProductId::parse("ordinary-product").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Internal,
            )),
            EndpointSnapshot::Initiative(pmc_domain::relationships::InitiativeSnapshot::new(
                InitiativeId::parse("ordinary-initiative").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Confidential,
            )),
            EndpointSnapshot::Roadmap(pmc_domain::relationships::RoadmapSnapshot::new(
                RoadmapId::parse("ordinary-roadmap").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Internal,
            )),
            EndpointSnapshot::Kpi(pmc_domain::relationships::KpiSnapshot::new(
                KpiId::parse("ordinary-kpi").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Internal,
            )),
            EndpointSnapshot::Project(pmc_domain::relationships::ProjectSnapshot::new(
                ProjectId::parse("ordinary-project").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Confidential,
            )),
            EndpointSnapshot::Milestone(pmc_domain::relationships::MilestoneSnapshot::new(
                MilestoneId::parse("ordinary-milestone").unwrap(),
                ProjectId::parse("ordinary-project").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Restricted,
            )),
        ]),
        AuditIds::default(),
        decoded,
    )
    .unwrap();
    let audits = restored.audit_events().len();
    let replay = restored
        .update_stakeholder(UpdateStakeholderDetails {
            id: StakeholderId::parse("ordinary-stakeholder").unwrap(),
            expected_version: AggregateVersion::initial(),
            name: pmc_domain::relationships::StakeholderName::parse("Synthetic owner").unwrap(),
            classification: Some(DataClassification::Restricted),
            context: ctx("ordinary-update-stakeholder"),
        })
        .unwrap();
    assert_eq!(replay.value().name().as_str(), "Synthetic owner");
    assert_eq!(replay.value().version(), AggregateVersion::new(2).unwrap());
    assert_eq!(restored.audit_events().len(), audits);
    let link = restored
        .link_portfolio_product(LinkPortfolioProduct {
            id: RelationshipId::parse("ordinary-rel-portfolio-product").unwrap(),
            portfolio_id: PortfolioId::parse("ordinary-portfolio").unwrap(),
            product_id: ProductId::parse("ordinary-product").unwrap(),
            expected_portfolio_version: AggregateVersion::initial(),
            expected_product_version: AggregateVersion::initial(),
            context: pmc_domain::relationships::OperationContext {
                idempotency_id: IdempotencyId::parse("sqlite-rel-ordinary-link-portfolio-product")
                    .unwrap(),
                correlation_id: CorrelationId::parse("ordinary-replay-changed-correlation")
                    .unwrap(),
            },
        })
        .unwrap();
    assert_eq!(
        link.value().id(),
        &RelationshipId::parse("ordinary-rel-portfolio-product").unwrap()
    );
    assert_eq!(restored.audit_events().len(), audits);
}

#[test]
fn sqlite_relationship_ordinary_decoder_rejects_missing_replay_audit() {
    let expected = ordinary_full_fixture().persistence_snapshot().unwrap();
    let db = TempLedger::new();
    SqliteProductLedger::open(&db.0).unwrap();
    let mut connection = open_test_connection(&db.0);
    persist_ordinary(&mut connection, &expected);
    connection
        .execute(
            "DELETE FROM relationship_replay_audits WHERE idempotency_id='sqlite-rel-ordinary-link-portfolio-product' AND ordinal=0",
            [],
        )
        .unwrap();
    let rejected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        decode_ordinary(&connection)
    }));
    assert!(rejected.is_err(), "missing replay audit must fail closed");
}

#[test]
fn sqlite_relationship_ordinary_decoder_rejects_endpoint_type_corruption() {
    let expected = ordinary_full_fixture().persistence_snapshot().unwrap();
    let db = TempLedger::new();
    SqliteProductLedger::open(&db.0).unwrap();
    let mut connection = open_test_connection(&db.0);
    persist_ordinary(&mut connection, &expected);
    let rejected = connection.execute(
        "UPDATE relationship_endpoints SET target_type='product' WHERE relationship_id='ordinary-rel-portfolio-product' AND ordinal=0",
        [],
    );
    assert!(
        rejected.is_err(),
        "endpoint type corruption must fail closed"
    );
}

#[test]
fn relationship_replay_operation_requires_one_exact_typed_payload_at_commit() {
    let db = TempLedger::new();
    SqliteProductLedger::open(&db.0).unwrap();
    let c = open_test_connection(&db.0);
    c.execute_batch("BEGIN; INSERT INTO relationship_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES('orphan','link','correlation',1,'relationship','relationship'); COMMIT;").expect_err("orphan pair");
}
#[test]
fn relationship_h2b_typed_payload_must_match_the_parent_result_kind() {
    let db = TempLedger::new();
    SqliteProductLedger::open(&db.0).unwrap();
    let c = open_test_connection(&db.0);
    c.execute_batch("BEGIN; INSERT INTO relationship_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,error_code,error_message_key,error_retryable) VALUES('mismatch','execute_remove','correlation',1,'rejection','SECURITY_POLICY_DENIED','relationship.removal.policy_denied',0); INSERT INTO relationship_h2b_command_results(idempotency_id,command_kind,result_kind,prepared_intent_id,actor,acknowledged_payload_digest,confirmation_digest,result_relationship_id) VALUES('mismatch','execute_remove','removal','prepared','head_of_products','ack','confirm','removed'); COMMIT;").expect_err("mismatch pair");
}

#[test]
fn relationship_replay_rejects_cross_family_payloads_and_invalid_stakeholder_rows() {
    let db = TempLedger::new();
    SqliteProductLedger::open(&db.0).unwrap();
    let connection = open_test_connection(&db.0);
    let cross_family = connection.execute_batch(
        "BEGIN;
         INSERT INTO relationship_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES('cross-family','create_stakeholder','cross-correlation',1,'stakeholder','stakeholder-cross');
         INSERT INTO relationship_stakeholder_command_results(idempotency_id,command_kind,command_id,result_id,command_name,command_stakeholder_kind,command_classification,command_provenance_kind,result_name,result_stakeholder_kind,result_classification,result_provenance_kind,result_version,result_created_at,result_updated_at,effect_scope) VALUES('cross-family','create_stakeholder','stakeholder-cross','stakeholder-cross','Synthetic','person','internal','user_entered','Synthetic','person','internal','user_entered',1,1,1,'complete');
         INSERT INTO relationship_link_command_results(idempotency_id,relationship_id,relationship_kind,result_version,result_classification,result_created_at,result_updated_at,effect_scope) VALUES('cross-family','relationship-cross','portfolio_product',1,'internal',1,1,'complete');
         COMMIT;",
    );
    assert!(
        cross_family.is_err(),
        "one operation cannot carry a second family payload"
    );
    connection.execute_batch("ROLLBACK").ok();

    for invalid_child in [
        "INSERT INTO relationship_stakeholder_command_results(idempotency_id,command_kind,command_id,result_id,command_name,command_stakeholder_kind,command_classification,command_provenance_kind,command_provenance_reference,result_name,result_stakeholder_kind,result_classification,result_provenance_kind,result_version,result_created_at,result_updated_at,effect_scope) VALUES('invalid-stakeholder','create_stakeholder','stakeholder-invalid','stakeholder-invalid','Synthetic','person','internal','user_entered','forbidden-reference','Synthetic','person','internal','user_entered',1,1,1,'complete')",
        "INSERT INTO relationship_stakeholder_command_results(idempotency_id,command_kind,command_id,result_id,command_name,command_expected_version,command_classification,result_name,result_stakeholder_kind,result_classification,result_provenance_kind,result_version,result_created_at,result_updated_at,effect_scope) VALUES('invalid-stakeholder','update_stakeholder','stakeholder-invalid','stakeholder-invalid','Synthetic',1,'restricted','Synthetic','person','public','user_entered',2,1,2,'complete')",
    ] {
        connection.execute_batch("BEGIN").unwrap();
        connection.execute("INSERT INTO relationship_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES('invalid-stakeholder',CASE WHEN ?1 LIKE '%update_stakeholder%' THEN 'update_stakeholder' ELSE 'create_stakeholder' END,'invalid-correlation',1,'stakeholder','stakeholder-invalid')", [invalid_child]).unwrap();
        assert!(connection.execute(invalid_child, []).is_err(), "invalid Stakeholder authority must fail closed");
        connection.execute_batch("ROLLBACK").unwrap();
    }
}

#[test]
fn successful_h2b_removal_must_acknowledge_the_exact_prepared_digest() {
    let db = TempLedger::new();
    SqliteProductLedger::open(&db.0).unwrap();
    let connection = open_test_connection(&db.0);
    let rejected = connection.execute_batch(
        "BEGIN;
         INSERT INTO prepared_intents(id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,confirmation_challenge,expires_at,created_at) VALUES('prepared-digest',1,'relationship.remove','expected-digest','restricted','allowed','not_cancellable_after_submit','head_of_products','REMOVE synthetic',100,1);
         INSERT INTO relationship_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES('execute-digest','execute_remove','execute-correlation',1,'removal','relationship-digest');
         INSERT INTO relationship_h2b_command_results(idempotency_id,command_kind,result_kind,prepared_intent_id,actor,acknowledged_payload_digest,confirmation_digest,result_relationship_id) VALUES('execute-digest','execute_remove','removal','prepared-digest','head_of_products','changed-digest','confirmation-digest','relationship-digest');
         COMMIT;",
    );
    assert!(
        rejected.is_err(),
        "successful removal must bind the exact prepared payload digest"
    );
    connection.execute_batch("ROLLBACK").ok();
}
