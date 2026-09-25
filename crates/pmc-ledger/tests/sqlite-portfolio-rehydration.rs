use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    audit::{AuditEvent, AuditEventIdSource},
    classification::DataClassification,
    identity::{
        AggregateVersion, AuditEventId, CorrelationId, IdempotencyId, KpiId, KpiObservationId,
        PortfolioId, ProductId, RoadmapId,
    },
    portfolio::{
        CreateKpiDefinition, CreateKpiObservation, CreatePortfolio, CreateProduct, CreateRoadmap,
        InMemoryPortfolioService, OperationContext, UpdateKpiDefinitionDetails,
        UpdateKpiObservationDetails, UpdatePortfolioDetails, UpdateProductDetails,
        UpdateRoadmapDetails,
    },
    provenance::{Provenance, ProvenanceReference},
    time::{Clock, UtcTimestamp},
    BoundedText, DomainValueError,
};
use pmc_ledger::sqlite::SqliteProductLedger;
use rusqlite::{params, Connection};

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

struct SyntheticLedger(PathBuf);

impl SyntheticLedger {
    fn new() -> Self {
        let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("synthetic clock follows epoch")
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "pmc-synthetic-portfolio-rehydration-{nonce}-{sequence}.sqlite3"
        )))
    }
}

impl Drop for SyntheticLedger {
    fn drop(&mut self) {
        for path in [
            self.0.clone(),
            PathBuf::from(format!("{}-wal", self.0.display())),
            PathBuf::from(format!("{}-shm", self.0.display())),
        ] {
            let _ = fs::remove_file(path);
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

#[derive(Default)]
struct AuditIds(u64);

impl AuditEventIdSource for AuditIds {
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        let id = AuditEventId::parse(format!("portfolio-sqlite-audit-{}", self.0))?;
        self.0 += 1;
        Ok(id)
    }
}

fn short(value: &str) -> BoundedText<160> {
    BoundedText::parse(value).unwrap()
}

fn long(value: &str) -> BoundedText<2_000> {
    BoundedText::parse(value).unwrap()
}

fn context(id: &str) -> OperationContext {
    OperationContext {
        idempotency_id: IdempotencyId::parse(id).unwrap(),
        correlation_id: CorrelationId::parse(format!("correlation-{id}")).unwrap(),
    }
}

fn provenance() -> Provenance {
    Provenance::SyntheticFixture(ProvenanceReference::parse("portfolio-sqlite-fixture").unwrap())
}

#[test]
fn all_portfolio_commands_cross_sqlite_and_replay_without_new_effects() {
    let mut service = InMemoryPortfolioService::new(FixedClock, AuditIds::default());
    let portfolio_id = PortfolioId::parse("portfolio-sqlite").unwrap();
    let product_id = ProductId::parse("product-sqlite").unwrap();
    let roadmap_id = RoadmapId::parse("roadmap-sqlite").unwrap();
    let kpi_id = KpiId::parse("kpi-sqlite").unwrap();
    let observation_a = KpiObservationId::parse("observation-a").unwrap();
    let observation_b = KpiObservationId::parse("observation-b").unwrap();

    service
        .create_portfolio(CreatePortfolio {
            id: portfolio_id.clone(),
            name: short("Portfolio one"),
            details: long("Portfolio details one"),
            classification: Some(DataClassification::Internal),
            provenance: provenance(),
            context: context("create-portfolio"),
        })
        .unwrap();
    service
        .update_portfolio_details(UpdatePortfolioDetails {
            id: portfolio_id,
            expected_version: AggregateVersion::initial(),
            name: short("Portfolio two"),
            details: long("Portfolio details two"),
            classification: Some(DataClassification::Confidential),
            context: context("update-portfolio"),
        })
        .unwrap();
    service
        .create_product(CreateProduct {
            id: product_id.clone(),
            name: short("Product one"),
            details: long("Product details one"),
            classification: Some(DataClassification::Internal),
            provenance: provenance(),
            context: context("create-product"),
        })
        .unwrap();
    service
        .update_product_details(UpdateProductDetails {
            id: product_id,
            expected_version: AggregateVersion::initial(),
            name: short("Product two"),
            details: long("Product details two"),
            classification: Some(DataClassification::Restricted),
            context: context("update-product"),
        })
        .unwrap();
    service
        .create_roadmap(CreateRoadmap {
            id: roadmap_id.clone(),
            name: short("Roadmap one"),
            details: long("Roadmap details one"),
            classification: Some(DataClassification::Internal),
            provenance: provenance(),
            context: context("create-roadmap"),
        })
        .unwrap();
    service
        .update_roadmap_details(UpdateRoadmapDetails {
            id: roadmap_id,
            expected_version: AggregateVersion::initial(),
            name: short("Roadmap two"),
            details: long("Roadmap details two"),
            classification: Some(DataClassification::Confidential),
            context: context("update-roadmap"),
        })
        .unwrap();
    service
        .create_kpi_definition(CreateKpiDefinition {
            id: kpi_id.clone(),
            name: short("KPI one"),
            definition: long("KPI definition one"),
            owner: short("Synthetic owner"),
            target: short("100"),
            cadence: short("weekly"),
            source: long("Synthetic KPI source"),
            classification: Some(DataClassification::Internal),
            provenance: provenance(),
            context: context("create-kpi"),
        })
        .unwrap();
    service
        .create_kpi_observation(CreateKpiObservation {
            id: observation_b,
            kpi_id: kpi_id.clone(),
            value: short("20"),
            observed_at: UtcTimestamp::from_unix_millis(20),
            source: long("Synthetic observation B"),
            classification: Some(DataClassification::Internal),
            provenance: provenance(),
            context: context("create-observation-b"),
        })
        .unwrap();
    service
        .create_kpi_observation(CreateKpiObservation {
            id: observation_a.clone(),
            kpi_id: kpi_id.clone(),
            value: short("10"),
            observed_at: UtcTimestamp::from_unix_millis(10),
            source: long("Synthetic observation A"),
            classification: Some(DataClassification::Internal),
            provenance: provenance(),
            context: context("create-observation-a"),
        })
        .unwrap();
    service
        .update_kpi_observation_details(UpdateKpiObservationDetails {
            id: observation_a,
            expected_version: AggregateVersion::initial(),
            value: short("11"),
            observed_at: UtcTimestamp::from_unix_millis(11),
            source: long("Synthetic observation A two"),
            classification: Some(DataClassification::Confidential),
            context: context("update-observation"),
        })
        .unwrap();
    let raised = service
        .update_kpi_definition_details(UpdateKpiDefinitionDetails {
            id: kpi_id,
            expected_version: AggregateVersion::initial(),
            name: short("KPI two"),
            definition: long("KPI definition two"),
            owner: short("Synthetic owner"),
            target: short("110"),
            cadence: short("weekly"),
            source: long("Synthetic KPI source two"),
            classification: Some(DataClassification::Restricted),
            context: context("update-kpi"),
        })
        .unwrap();

    let snapshot = service.persistence_snapshot();
    assert_eq!(snapshot.replay().len(), 11);
    assert_eq!(
        snapshot
            .replay()
            .last()
            .unwrap()
            .derived_kpi_observation_mutations()
            .len(),
        2
    );

    let ledger = SyntheticLedger::new();
    drop(SqliteProductLedger::open(&ledger.0).expect("synthetic ledger initializes"));
    let mut connection = Connection::open(&ledger.0).expect("synthetic ledger opens");
    persist_snapshot(&mut connection, &snapshot);
    connection.execute("INSERT INTO audit_events VALUES('unrelated-delivery-audit',100,'head_of_products','portfolio','delivery.project.created','project','unrelated-project','unrelated-correlation','not_required','not_required','succeeded','complete')", []).expect("shared Ledger may contain another family audit");
    connection.execute("INSERT INTO audit_effects VALUES('unrelated-delivery-audit',0,'create_project','complete','project','unrelated-project')", []).expect("unrelated effect is valid shared history");
    let decoded = decode_snapshot(&connection);
    let mut restored = InMemoryPortfolioService::rehydrate(FixedClock, AuditIds(100), decoded);
    let audit_count = restored.audit_events().len();
    let replayed = restored
        .update_kpi_definition_details(UpdateKpiDefinitionDetails {
            id: KpiId::parse("kpi-sqlite").unwrap(),
            expected_version: AggregateVersion::initial(),
            name: short("KPI two"),
            definition: long("KPI definition two"),
            owner: short("Synthetic owner"),
            target: short("110"),
            cadence: short("weekly"),
            source: long("Synthetic KPI source two"),
            classification: Some(DataClassification::Restricted),
            context: context("update-kpi"),
        })
        .unwrap();
    assert_eq!(replayed, raised);
    assert_eq!(restored.audit_events().len(), audit_count);

    let original_mutation_audit: String = connection.query_row("SELECT audit_event_id FROM portfolio_derived_kpi_observation_mutations WHERE operation='update_kpi_definition' AND ordinal=1",[],|row|row.get(0)).unwrap();
    connection.execute("UPDATE portfolio_derived_kpi_observation_mutations SET ordinal=7 WHERE operation='update_kpi_definition' AND ordinal=1",[]).unwrap();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode_snapshot(
            &connection
        )))
        .is_err(),
        "mutation order corruption fails closed"
    );
    connection.execute("UPDATE portfolio_derived_kpi_observation_mutations SET ordinal=1 WHERE operation='update_kpi_definition' AND ordinal=7",[]).unwrap();
    connection.execute("PRAGMA foreign_keys=OFF", []).unwrap();
    connection.execute("UPDATE portfolio_derived_kpi_observation_mutations SET audit_event_id=(SELECT audit_event_id FROM portfolio_idempotency_outcome_audits WHERE operation='update_kpi_definition' AND ordinal=0) WHERE operation='update_kpi_definition' AND ordinal=1",[]).unwrap();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode_snapshot(
            &connection
        )))
        .is_err(),
        "mutation audit corruption fails closed"
    );
    connection.execute("UPDATE portfolio_derived_kpi_observation_mutations SET audit_event_id=?1 WHERE operation='update_kpi_definition' AND ordinal=1",[original_mutation_audit]).unwrap();
    connection
        .execute(
            "UPDATE kpi_observations SET kpi_id='missing-kpi' WHERE id='observation-a'",
            [],
        )
        .unwrap();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode_snapshot(
            &connection
        )))
        .is_err(),
        "missing KPI parent fails closed"
    );
    connection
        .execute(
            "UPDATE kpi_observations SET kpi_id='kpi-sqlite' WHERE id='observation-a'",
            [],
        )
        .unwrap();
    connection.execute("UPDATE portfolio_command_results SET result_updated_at=result_updated_at+1 WHERE operation='update_product'",[]).unwrap();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode_snapshot(
            &connection
        )))
        .is_err(),
        "non-observation result-time corruption fails closed"
    );
}

fn persist_snapshot(
    connection: &mut Connection,
    snapshot: &pmc_domain::portfolio::PortfolioPersistenceSnapshot,
) {
    let transaction = connection.transaction().unwrap();
    for record in snapshot.portfolios() {
        transaction
            .execute(
                "INSERT INTO aggregate_registry VALUES(?1,'portfolio',?2,?3,100,100)",
                params![
                    record.id.as_str(),
                    i64::try_from(record.version.get()).unwrap(),
                    record.classification.as_persisted()
                ],
            )
            .unwrap();
        transaction.execute("INSERT INTO portfolios(id,name,details,provenance_kind,provenance_reference) VALUES(?1,?2,?3,?4,?5)", params![record.id.as_str(),record.name.as_str(),record.details.as_str(),record.provenance.kind_persisted(),record.provenance.reference().map(|v|v.as_str())]).unwrap();
    }
    for record in snapshot.products() {
        transaction
            .execute(
                "INSERT INTO aggregate_registry VALUES(?1,'product',?2,?3,100,100)",
                params![
                    record.id.as_str(),
                    i64::try_from(record.version.get()).unwrap(),
                    record.classification.as_persisted()
                ],
            )
            .unwrap();
        transaction.execute("INSERT INTO products(id,name,details,provenance_kind,provenance_reference) VALUES(?1,?2,?3,?4,?5)", params![record.id.as_str(),record.name.as_str(),record.details.as_str(),record.provenance.kind_persisted(),record.provenance.reference().map(|v|v.as_str())]).unwrap();
    }
    for record in snapshot.roadmaps() {
        transaction
            .execute(
                "INSERT INTO aggregate_registry VALUES(?1,'roadmap',?2,?3,100,100)",
                params![
                    record.id.as_str(),
                    i64::try_from(record.version.get()).unwrap(),
                    record.classification.as_persisted()
                ],
            )
            .unwrap();
        transaction.execute("INSERT INTO roadmaps(id,name,details,provenance_kind,provenance_reference) VALUES(?1,?2,?3,?4,?5)", params![record.id.as_str(),record.name.as_str(),record.details.as_str(),record.provenance.kind_persisted(),record.provenance.reference().map(|v|v.as_str())]).unwrap();
    }
    for record in snapshot.kpi_definitions() {
        transaction
            .execute(
                "INSERT INTO aggregate_registry VALUES(?1,'kpi_definition',?2,?3,100,100)",
                params![
                    record.id.as_str(),
                    i64::try_from(record.version.get()).unwrap(),
                    record.classification.as_persisted()
                ],
            )
            .unwrap();
        transaction.execute("INSERT INTO kpi_definitions(id,name,definition,owner,target,cadence,source,provenance_kind,provenance_reference) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)", params![record.id.as_str(),record.name.as_str(),record.definition.as_str(),record.owner.as_str(),record.target.as_str(),record.cadence.as_str(),record.source.as_str(),record.provenance.kind_persisted(),record.provenance.reference().map(|v|v.as_str())]).unwrap();
    }
    for record in snapshot.kpi_observations() {
        transaction
            .execute(
                "INSERT INTO aggregate_registry VALUES(?1,'kpi_observation',?2,?3,?4,?5)",
                params![
                    record.id.as_str(),
                    i64::try_from(record.version.get()).unwrap(),
                    record.classification.as_persisted(),
                    record.created_at.unix_millis(),
                    record.updated_at.unix_millis()
                ],
            )
            .unwrap();
        transaction.execute("INSERT INTO kpi_observations(id,kpi_id,value,observed_at,source,provenance_kind,provenance_reference) VALUES(?1,?2,?3,?4,?5,?6,?7)", params![record.id.as_str(),record.kpi_id.as_str(),record.value.as_str(),record.observed_at.unix_millis(),record.source.as_str(),record.provenance.kind_persisted(),record.provenance.reference().map(|v|v.as_str())]).unwrap();
    }
    persist_audits(&transaction, snapshot.audits());
    let mut result_times = HashMap::new();
    for capsule in snapshot.replay() {
        persist_capsule(&transaction, capsule, &mut result_times);
    }
    transaction.commit().unwrap();
}

fn persist_audits(transaction: &rusqlite::Transaction<'_>, audits: &[AuditEvent]) {
    use pmc_domain::audit::AuditTarget;
    for audit in audits {
        let (target_type, target_id) = match audit.target() {
            AuditTarget::Portfolio(v) => ("portfolio", v.as_str()),
            AuditTarget::Product(v) => ("product", v.as_str()),
            AuditTarget::Roadmap(v) => ("roadmap", v.as_str()),
            AuditTarget::Kpi(v) => ("kpi_definition", v.as_str()),
            AuditTarget::KpiObservation(v) => ("kpi_observation", v.as_str()),
            _ => panic!("unexpected Portfolio audit target"),
        };
        transaction
            .execute(
                "INSERT INTO audit_events VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                params![
                    audit.id().as_str(),
                    audit.occurred_at().unix_millis(),
                    audit.actor().as_persisted(),
                    audit.module().as_persisted(),
                    audit.code().as_str(),
                    target_type,
                    target_id,
                    audit.correlation_id().as_str(),
                    audit.policy_outcome().as_persisted(),
                    audit.approval_outcome().as_persisted(),
                    audit.execution_outcome().as_persisted(),
                    audit.effect_scope().as_persisted()
                ],
            )
            .unwrap();
        for (ordinal, effect) in audit.actual_effects().iter().enumerate() {
            transaction.execute("INSERT INTO audit_effects(audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES(?1,?2,?3,?4,?5,?6)", params![audit.id().as_str(),i64::try_from(ordinal).unwrap(),effect.as_str(),audit.effect_scope().as_persisted(),target_type,target_id]).unwrap();
        }
    }
}

#[derive(Default)]
struct ReplayFields {
    operation: &'static str,
    target_id: String,
    expected_version: Option<i64>,
    name: Option<String>,
    details: Option<String>,
    definition: Option<String>,
    owner: Option<String>,
    target: Option<String>,
    cadence: Option<String>,
    source: Option<String>,
    kpi_id: Option<String>,
    value: Option<String>,
    observed_at: Option<i64>,
    classification: Option<String>,
    provenance_kind: Option<String>,
    provenance_reference: Option<String>,
    result_kind: &'static str,
    result_name: Option<String>,
    result_details: Option<String>,
    result_definition: Option<String>,
    result_owner: Option<String>,
    result_target: Option<String>,
    result_cadence: Option<String>,
    result_source: Option<String>,
    result_kpi_id: Option<String>,
    result_value: Option<String>,
    result_observed_at: Option<i64>,
    result_classification: String,
    result_provenance_kind: String,
    result_provenance_reference: Option<String>,
    result_version: i64,
    result_created_at: i64,
    result_updated_at: i64,
}

fn provenance_fields(value: &Provenance) -> (String, Option<String>) {
    (
        value.kind_persisted().to_owned(),
        value.reference().map(|v| v.as_str().to_owned()),
    )
}

fn persist_capsule(
    transaction: &rusqlite::Transaction<'_>,
    capsule: &pmc_domain::portfolio::PortfolioReplayCapsule,
    result_times: &mut HashMap<String, (i64, i64)>,
) {
    use pmc_domain::portfolio::{CommandIdentity, PortfolioPersistenceResult};
    let mut fields = ReplayFields::default();
    match capsule.command() {
        CommandIdentity::CreatePortfolio {
            id,
            name,
            details,
            classification,
            provenance,
        } => {
            fields.operation = "create_portfolio";
            fields.target_id = id.as_str().into();
            fields.name = Some(name.as_str().into());
            fields.details = Some(details.as_str().into());
            fields.classification = classification.map(|v| v.as_persisted().into());
            let p = provenance_fields(provenance);
            fields.provenance_kind = Some(p.0);
            fields.provenance_reference = p.1;
        }
        CommandIdentity::UpdatePortfolio {
            id,
            expected_version,
            name,
            details,
            classification,
        } => {
            fields.operation = "update_portfolio";
            fields.target_id = id.as_str().into();
            fields.expected_version = Some(expected_version.get().try_into().unwrap());
            fields.name = Some(name.as_str().into());
            fields.details = Some(details.as_str().into());
            fields.classification = classification.map(|v| v.as_persisted().into());
        }
        CommandIdentity::CreateProduct {
            id,
            name,
            details,
            classification,
            provenance,
        } => {
            fields.operation = "create_product";
            fields.target_id = id.as_str().into();
            fields.name = Some(name.as_str().into());
            fields.details = Some(details.as_str().into());
            fields.classification = classification.map(|v| v.as_persisted().into());
            let p = provenance_fields(provenance);
            fields.provenance_kind = Some(p.0);
            fields.provenance_reference = p.1;
        }
        CommandIdentity::UpdateProduct {
            id,
            expected_version,
            name,
            details,
            classification,
        } => {
            fields.operation = "update_product";
            fields.target_id = id.as_str().into();
            fields.expected_version = Some(expected_version.get().try_into().unwrap());
            fields.name = Some(name.as_str().into());
            fields.details = Some(details.as_str().into());
            fields.classification = classification.map(|v| v.as_persisted().into());
        }
        CommandIdentity::CreateRoadmap {
            id,
            name,
            details,
            classification,
            provenance,
        } => {
            fields.operation = "create_roadmap";
            fields.target_id = id.as_str().into();
            fields.name = Some(name.as_str().into());
            fields.details = Some(details.as_str().into());
            fields.classification = classification.map(|v| v.as_persisted().into());
            let p = provenance_fields(provenance);
            fields.provenance_kind = Some(p.0);
            fields.provenance_reference = p.1;
        }
        CommandIdentity::UpdateRoadmap {
            id,
            expected_version,
            name,
            details,
            classification,
        } => {
            fields.operation = "update_roadmap";
            fields.target_id = id.as_str().into();
            fields.expected_version = Some(expected_version.get().try_into().unwrap());
            fields.name = Some(name.as_str().into());
            fields.details = Some(details.as_str().into());
            fields.classification = classification.map(|v| v.as_persisted().into());
        }
        CommandIdentity::CreateKpi {
            id,
            name,
            definition,
            owner,
            target,
            cadence,
            source,
            classification,
            provenance,
        } => {
            fields.operation = "create_kpi_definition";
            fields.target_id = id.as_str().into();
            fields.name = Some(name.as_str().into());
            fields.definition = Some(definition.as_str().into());
            fields.owner = Some(owner.as_str().into());
            fields.target = Some(target.as_str().into());
            fields.cadence = Some(cadence.as_str().into());
            fields.source = Some(source.as_str().into());
            fields.classification = classification.map(|v| v.as_persisted().into());
            let p = provenance_fields(provenance);
            fields.provenance_kind = Some(p.0);
            fields.provenance_reference = p.1;
        }
        CommandIdentity::UpdateKpi {
            id,
            expected_version,
            name,
            definition,
            owner,
            target,
            cadence,
            source,
            classification,
        } => {
            fields.operation = "update_kpi_definition";
            fields.target_id = id.as_str().into();
            fields.expected_version = Some(expected_version.get().try_into().unwrap());
            fields.name = Some(name.as_str().into());
            fields.definition = Some(definition.as_str().into());
            fields.owner = Some(owner.as_str().into());
            fields.target = Some(target.as_str().into());
            fields.cadence = Some(cadence.as_str().into());
            fields.source = Some(source.as_str().into());
            fields.classification = classification.map(|v| v.as_persisted().into());
        }
        CommandIdentity::CreateObservation {
            id,
            kpi_id,
            value,
            observed_at,
            source,
            classification,
            provenance,
        } => {
            fields.operation = "create_kpi_observation";
            fields.target_id = id.as_str().into();
            fields.kpi_id = Some(kpi_id.as_str().into());
            fields.value = Some(value.as_str().into());
            fields.observed_at = Some(observed_at.unix_millis());
            fields.source = Some(source.as_str().into());
            fields.classification = classification.map(|v| v.as_persisted().into());
            let p = provenance_fields(provenance);
            fields.provenance_kind = Some(p.0);
            fields.provenance_reference = p.1;
        }
        CommandIdentity::UpdateObservation {
            id,
            expected_version,
            value,
            observed_at,
            source,
            classification,
        } => {
            fields.operation = "update_kpi_observation";
            fields.target_id = id.as_str().into();
            fields.expected_version = Some(expected_version.get().try_into().unwrap());
            fields.value = Some(value.as_str().into());
            fields.observed_at = Some(observed_at.unix_millis());
            fields.source = Some(source.as_str().into());
            fields.classification = classification.map(|v| v.as_persisted().into());
        }
        // H2a lowering is domain-only here -- `pmc-ledger`'s production
        // `portfolio_repository.rs` does not yet call any of the ten
        // Prepare/ApproveAndExecute Lower<X>Classification methods across
        // the five Portfolio-family record types, so this
        // fixture-generating test never actually produces any of these
        // capsule shapes. Left as a loud failure (not a silent `_ => {}`)
        // so a future ledger-persistence change that starts producing one is
        // forced to add the real field mapping here rather than silently
        // falling through.
        CommandIdentity::PrepareLowerPortfolioClassification { .. }
        | CommandIdentity::ApproveAndExecuteLowerPortfolioClassification { .. }
        | CommandIdentity::PrepareLowerProductClassification { .. }
        | CommandIdentity::ApproveAndExecuteLowerProductClassification { .. }
        | CommandIdentity::PrepareLowerRoadmapClassification { .. }
        | CommandIdentity::ApproveAndExecuteLowerRoadmapClassification { .. }
        | CommandIdentity::PrepareLowerKpiClassification { .. }
        | CommandIdentity::ApproveAndExecuteLowerKpiClassification { .. }
        | CommandIdentity::PrepareLowerKpiObservationClassification { .. }
        | CommandIdentity::ApproveAndExecuteLowerKpiObservationClassification { .. } => {
            unreachable!(
                "Lower Data Classification is not yet wired into pmc-ledger's portfolio repository"
            )
        }
    }
    let outcome_audit;
    match capsule.result() {
        PortfolioPersistenceResult::Portfolio { outcome } => {
            let v = &outcome.record;
            fields.result_kind = "portfolio";
            fields.result_name = Some(v.name.as_str().into());
            fields.result_details = Some(v.details.as_str().into());
            set_result_common(
                &mut fields,
                v.classification,
                &v.provenance,
                v.version,
                0,
                0,
            );
            outcome_audit = &outcome.audit_event;
        }
        PortfolioPersistenceResult::Product { outcome } => {
            let v = &outcome.record;
            fields.result_kind = "product";
            fields.result_name = Some(v.name.as_str().into());
            fields.result_details = Some(v.details.as_str().into());
            set_result_common(
                &mut fields,
                v.classification,
                &v.provenance,
                v.version,
                0,
                0,
            );
            outcome_audit = &outcome.audit_event;
        }
        PortfolioPersistenceResult::Roadmap { outcome } => {
            let v = &outcome.record;
            fields.result_kind = "roadmap";
            fields.result_name = Some(v.name.as_str().into());
            fields.result_details = Some(v.details.as_str().into());
            set_result_common(
                &mut fields,
                v.classification,
                &v.provenance,
                v.version,
                0,
                0,
            );
            outcome_audit = &outcome.audit_event;
        }
        PortfolioPersistenceResult::KpiDefinition { outcome } => {
            let v = &outcome.record;
            fields.result_kind = "kpi_definition";
            fields.result_name = Some(v.name.as_str().into());
            fields.result_definition = Some(v.definition.as_str().into());
            fields.result_owner = Some(v.owner.as_str().into());
            fields.result_target = Some(v.target.as_str().into());
            fields.result_cadence = Some(v.cadence.as_str().into());
            fields.result_source = Some(v.source.as_str().into());
            set_result_common(
                &mut fields,
                v.classification,
                &v.provenance,
                v.version,
                0,
                0,
            );
            outcome_audit = &outcome.audit_event;
        }
        PortfolioPersistenceResult::KpiObservation { outcome } => {
            let v = &outcome.record;
            fields.result_kind = "kpi_observation";
            fields.result_kpi_id = Some(v.kpi_id.as_str().into());
            fields.result_value = Some(v.value.as_str().into());
            fields.result_observed_at = Some(v.observed_at.unix_millis());
            fields.result_source = Some(v.source.as_str().into());
            set_result_common(
                &mut fields,
                v.classification,
                &v.provenance,
                v.version,
                v.created_at.unix_millis(),
                v.updated_at.unix_millis(),
            );
            outcome_audit = &outcome.audit_event;
        }
    }
    if fields.result_kind == "kpi_observation" {
        result_times.insert(
            fields.target_id.clone(),
            (fields.result_created_at, fields.result_updated_at),
        );
    } else if fields.operation.starts_with("create_") {
        let occurred_at = outcome_audit.occurred_at().unix_millis();
        fields.result_created_at = occurred_at;
        fields.result_updated_at = occurred_at;
        result_times.insert(fields.target_id.clone(), (occurred_at, occurred_at));
    } else {
        let (created_at, previous_updated_at) = result_times
            .get(&fields.target_id)
            .copied()
            .expect("update follows its create");
        fields.result_created_at = created_at;
        fields.result_updated_at =
            previous_updated_at.max(outcome_audit.occurred_at().unix_millis());
        result_times.insert(
            fields.target_id.clone(),
            (fields.result_created_at, fields.result_updated_at),
        );
    }
    transaction
        .execute(
            "INSERT INTO idempotency_outcomes VALUES('portfolio',?1,?2,?3,'succeeded',?4,?5)",
            params![
                fields.operation,
                capsule.idempotency_id().as_str(),
                format!("synthetic-digest-{}", capsule.operation_ordinal()),
                fields.target_id.as_str(),
                outcome_audit.occurred_at().unix_millis()
            ],
        )
        .unwrap();
    transaction
        .execute(
            "INSERT INTO portfolio_idempotency_outcomes VALUES('portfolio',?1,?2,?1,?3,?4,'succeeded',?5)",
            params![
                fields.operation,
                capsule.idempotency_id().as_str(),
                capsule.correlation_id().as_str(),
                i64::try_from(capsule.operation_ordinal()).unwrap(),
                fields.target_id,
            ],
        )
        .unwrap();
    transaction.execute("INSERT INTO portfolio_command_results(namespace,operation,idempotency_id,command_kind,command_target_id,command_expected_version,command_name,command_details,command_definition,command_owner,command_target,command_cadence,command_source,command_kpi_id,command_value,command_observed_at,command_classification,command_provenance_kind,command_provenance_reference,result_kind,result_id,result_name,result_details,result_definition,result_owner,result_target,result_cadence,result_source,result_kpi_id,result_value,result_observed_at,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at) VALUES('portfolio',?1,?2,?1,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?3,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?31,?32,?33,?34)", params![fields.operation,capsule.idempotency_id().as_str(),fields.target_id,fields.expected_version,fields.name,fields.details,fields.definition,fields.owner,fields.target,fields.cadence,fields.source,fields.kpi_id,fields.value,fields.observed_at,fields.classification,fields.provenance_kind,fields.provenance_reference,fields.result_kind,fields.result_name,fields.result_details,fields.result_definition,fields.result_owner,fields.result_target,fields.result_cadence,fields.result_source,fields.result_kpi_id,fields.result_value,fields.result_observed_at,fields.result_classification,fields.result_provenance_kind,fields.result_provenance_reference,fields.result_version,fields.result_created_at,fields.result_updated_at]).unwrap();
    for (ordinal, audit_id) in capsule.audit_event_ids().iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO portfolio_idempotency_outcome_audits VALUES('portfolio',?1,?2,?3,?4)",
                params![
                    fields.operation,
                    capsule.idempotency_id().as_str(),
                    i64::try_from(ordinal).unwrap(),
                    audit_id.as_str()
                ],
            )
            .unwrap();
    }
    for (ordinal, mutation) in capsule
        .derived_kpi_observation_mutations()
        .iter()
        .enumerate()
    {
        transaction.execute("INSERT INTO portfolio_derived_kpi_observation_mutations VALUES('portfolio',?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)", params![fields.operation,capsule.idempotency_id().as_str(),i64::try_from(ordinal).unwrap(),mutation.observation_id().as_str(),i64::try_from(mutation.previous_version().get()).unwrap(),i64::try_from(mutation.resulting_version().get()).unwrap(),mutation.previous_classification().as_persisted(),mutation.resulting_classification().as_persisted(),mutation.previous_updated_at().unix_millis(),mutation.resulting_updated_at().unix_millis(),mutation.audit_event_id().as_str()]).unwrap();
    }
}

fn set_result_common(
    fields: &mut ReplayFields,
    classification: DataClassification,
    provenance: &Provenance,
    version: AggregateVersion,
    created_at: i64,
    updated_at: i64,
) {
    fields.result_classification = classification.as_persisted().into();
    let p = provenance_fields(provenance);
    fields.result_provenance_kind = p.0;
    fields.result_provenance_reference = p.1;
    fields.result_version = version.get().try_into().unwrap();
    fields.result_created_at = created_at;
    fields.result_updated_at = updated_at;
}

fn decode_snapshot(connection: &Connection) -> pmc_domain::portfolio::PortfolioPersistenceSnapshot {
    use pmc_domain::portfolio::{
        KpiDefinitionRecord, KpiObservationRecord, PortfolioRecord, ProductRecord, RoadmapRecord,
    };
    let portfolios = connection.prepare("SELECT p.id,p.name,p.details,r.classification,p.provenance_kind,p.provenance_reference,r.version FROM portfolios p JOIN aggregate_registry r ON r.id=p.id ORDER BY p.id").unwrap().query_map([],|row| Ok(PortfolioRecord { id: PortfolioId::parse(row.get::<_,String>(0)?).unwrap(), name: short(&row.get::<_,String>(1)?), details: long(&row.get::<_,String>(2)?), classification: classification(&row.get::<_,String>(3)?), provenance: decode_provenance(&row.get::<_,String>(4)?,row.get::<_,Option<String>>(5)?), version: version(row.get(6)?) })).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    let products = connection.prepare("SELECT p.id,p.name,p.details,r.classification,p.provenance_kind,p.provenance_reference,r.version FROM products p JOIN aggregate_registry r ON r.id=p.id ORDER BY p.id").unwrap().query_map([],|row| Ok(ProductRecord { id: ProductId::parse(row.get::<_,String>(0)?).unwrap(), name: short(&row.get::<_,String>(1)?), details: long(&row.get::<_,String>(2)?), classification: classification(&row.get::<_,String>(3)?), provenance: decode_provenance(&row.get::<_,String>(4)?,row.get::<_,Option<String>>(5)?), version: version(row.get(6)?) })).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    let roadmaps = connection.prepare("SELECT p.id,p.name,p.details,r.classification,p.provenance_kind,p.provenance_reference,r.version FROM roadmaps p JOIN aggregate_registry r ON r.id=p.id ORDER BY p.id").unwrap().query_map([],|row| Ok(RoadmapRecord { id: RoadmapId::parse(row.get::<_,String>(0)?).unwrap(), name: short(&row.get::<_,String>(1)?), details: long(&row.get::<_,String>(2)?), classification: classification(&row.get::<_,String>(3)?), provenance: decode_provenance(&row.get::<_,String>(4)?,row.get::<_,Option<String>>(5)?), version: version(row.get(6)?) })).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    let kpi_definitions = connection.prepare("SELECT p.id,p.name,p.definition,p.owner,p.target,p.cadence,p.source,r.classification,p.provenance_kind,p.provenance_reference,r.version FROM kpi_definitions p JOIN aggregate_registry r ON r.id=p.id ORDER BY p.id").unwrap().query_map([],|row| Ok(KpiDefinitionRecord { id: KpiId::parse(row.get::<_,String>(0)?).unwrap(), name: short(&row.get::<_,String>(1)?), definition: long(&row.get::<_,String>(2)?), owner: short(&row.get::<_,String>(3)?), target: short(&row.get::<_,String>(4)?), cadence: short(&row.get::<_,String>(5)?), source: long(&row.get::<_,String>(6)?), classification: classification(&row.get::<_,String>(7)?), provenance: decode_provenance(&row.get::<_,String>(8)?,row.get::<_,Option<String>>(9)?), version: version(row.get(10)?) })).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    let kpi_observations = connection.prepare("SELECT p.id,p.kpi_id,p.value,p.observed_at,p.source,r.classification,p.provenance_kind,p.provenance_reference,r.version,r.created_at,r.updated_at FROM kpi_observations p JOIN aggregate_registry r ON r.id=p.id ORDER BY p.id").unwrap().query_map([],|row| Ok(KpiObservationRecord { id: KpiObservationId::parse(row.get::<_,String>(0)?).unwrap(), kpi_id: KpiId::parse(row.get::<_,String>(1)?).unwrap(), value: short(&row.get::<_,String>(2)?), observed_at: UtcTimestamp::from_unix_millis(row.get(3)?), source: long(&row.get::<_,String>(4)?), classification: classification(&row.get::<_,String>(5)?), provenance: decode_provenance(&row.get::<_,String>(6)?,row.get::<_,Option<String>>(7)?), version: version(row.get(8)?), created_at: UtcTimestamp::from_unix_millis(row.get(9)?), updated_at: UtcTimestamp::from_unix_millis(row.get(10)?) })).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    let audits = decode_audits(connection);
    let audit_map = audits
        .iter()
        .map(|v| (v.id().as_str().to_owned(), v.clone()))
        .collect::<HashMap<_, _>>();
    validate_result_times(connection);
    let replay = decode_replay(connection, &audit_map);
    pmc_domain::portfolio::PortfolioPersistenceSnapshot::validate(
        portfolios,
        products,
        roadmaps,
        kpi_definitions,
        kpi_observations,
        replay,
        audits,
    )
    .expect("SQLite-only decoded history validates")
}

fn validate_result_times(connection: &Connection) {
    let rows=connection.prepare("SELECT o.operation,o.idempotency_id,r.result_id,r.result_created_at,r.result_updated_at,e.occurred_at FROM portfolio_idempotency_outcomes o JOIN portfolio_command_results r USING(namespace,operation,idempotency_id) JOIN portfolio_idempotency_outcome_audits a USING(namespace,operation,idempotency_id) JOIN audit_events e ON e.id=a.audit_event_id WHERE a.ordinal=0 ORDER BY o.operation_ordinal").unwrap().query_map([],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,i64>(3)?,row.get::<_,i64>(4)?,row.get::<_,i64>(5)?))).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    let mut timeline = HashMap::new();
    for (operation, idempotency_id, id, created_at, updated_at, occurred_at) in rows {
        if operation.starts_with("create_") {
            assert!(
                timeline
                    .insert(id.clone(), (created_at, updated_at))
                    .is_none()
                    && created_at == updated_at
                    && updated_at == occurred_at,
                "create result time equals its authoritative audit time"
            );
        } else {
            let (previous_created, previous_updated) = timeline
                .get(&id)
                .copied()
                .expect("update result requires created history");
            assert_eq!(created_at, previous_created, "created_at is immutable");
            assert_eq!(
                updated_at,
                previous_updated.max(occurred_at),
                "updated_at follows exact monotonic audit chronology"
            );
            timeline.insert(id.clone(), (created_at, updated_at));
        }
        let mutations=connection.prepare("SELECT m.observation_id,m.previous_updated_at,m.resulting_updated_at,e.occurred_at FROM portfolio_derived_kpi_observation_mutations m JOIN audit_events e ON e.id=m.audit_event_id WHERE m.operation=?1 AND m.idempotency_id=?2 ORDER BY m.ordinal").unwrap().query_map(params![operation,idempotency_id],|row|Ok((row.get::<_,String>(0)?,row.get::<_,i64>(1)?,row.get::<_,i64>(2)?,row.get::<_,i64>(3)?))).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
        for (observation_id, previous_updated, resulting_updated, mutation_occurred_at) in mutations
        {
            let (observation_created, current_updated) = timeline
                .get(&observation_id)
                .copied()
                .expect("derived mutation requires observation history");
            assert_eq!(
                previous_updated, current_updated,
                "derived mutation starts at exact persisted time"
            );
            assert_eq!(
                resulting_updated,
                previous_updated.max(mutation_occurred_at),
                "derived mutation follows exact monotonic audit chronology"
            );
            timeline.insert(observation_id, (observation_created, resulting_updated));
        }
    }
    for (id, (created_at, updated_at)) in timeline {
        let current: (i64, i64) = connection
            .query_row(
                "SELECT created_at,updated_at FROM aggregate_registry WHERE id=?1",
                [&id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            current,
            (created_at, updated_at),
            "final aggregate times equal exact replay lineage"
        );
    }
}

fn classification(value: &str) -> DataClassification {
    DataClassification::from_persisted(value).unwrap()
}

fn version(value: i64) -> AggregateVersion {
    AggregateVersion::new(u64::try_from(value).unwrap()).unwrap()
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
        _ => panic!("closed provenance"),
    }
}

fn decode_audits(connection: &Connection) -> Vec<AuditEvent> {
    use pmc_domain::audit::{
        AuditAction, AuditActor, AuditApprovalOutcome, AuditDisposition, AuditEffectCode,
        AuditEffectScope, AuditEventCode, AuditExecutionOutcome, AuditModule, AuditPolicyOutcome,
        AuditTarget,
    };
    let mut statement=connection.prepare("SELECT e.id,e.occurred_at,e.actor,e.module,e.event_code,e.target_type,e.target_id,e.correlation_id,e.policy_outcome,e.approval_outcome,e.execution_outcome,e.effect_scope FROM portfolio_idempotency_outcomes o JOIN portfolio_idempotency_outcome_audits a USING(namespace,operation,idempotency_id) JOIN audit_events e ON e.id=a.audit_event_id ORDER BY o.operation_ordinal,a.ordinal").unwrap();
    statement.query_map([],|row| {
        let id:String=row.get(0)?; let target_type:String=row.get(5)?; let target_id:String=row.get(6)?;
        let target=match target_type.as_str() { "portfolio"=>AuditTarget::Portfolio(PortfolioId::parse(target_id).unwrap()), "product"=>AuditTarget::Product(ProductId::parse(target_id).unwrap()), "roadmap"=>AuditTarget::Roadmap(RoadmapId::parse(target_id).unwrap()), "kpi_definition"=>AuditTarget::Kpi(KpiId::parse(target_id).unwrap()), "kpi_observation"=>AuditTarget::KpiObservation(KpiObservationId::parse(target_id).unwrap()), _=>panic!("closed target") };
        let effects=connection.prepare("SELECT effect_code FROM audit_effects WHERE audit_event_id=?1 ORDER BY ordinal").unwrap().query_map([&id],|effect| Ok(AuditEffectCode::parse(effect.get::<_,String>(0)?).unwrap())).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
        Ok(AuditEvent::new(AuditEventId::parse(id).unwrap(),UtcTimestamp::from_unix_millis(row.get(1)?),AuditActor::from_persisted(&row.get::<_,String>(2)?).unwrap(),AuditAction::new(AuditModule::from_persisted(&row.get::<_,String>(3)?).unwrap(),AuditEventCode::parse(row.get::<_,String>(4)?).unwrap(),target),CorrelationId::parse(row.get::<_,String>(7)?).unwrap(),AuditDisposition::new(AuditPolicyOutcome::from_persisted(&row.get::<_,String>(8)?).unwrap(),AuditApprovalOutcome::from_persisted(&row.get::<_,String>(9)?).unwrap(),AuditExecutionOutcome::from_persisted(&row.get::<_,String>(10)?).unwrap(),AuditEffectScope::from_persisted(&row.get::<_,String>(11)?).unwrap(),effects).unwrap()))
    }).unwrap().collect::<Result<Vec<_>,_>>().unwrap()
}

fn decode_replay(
    connection: &Connection,
    audits: &HashMap<String, AuditEvent>,
) -> Vec<pmc_domain::portfolio::PortfolioReplayCapsule> {
    let keys=connection.prepare("SELECT operation,idempotency_id,correlation_id,operation_ordinal FROM portfolio_idempotency_outcomes ORDER BY operation_ordinal").unwrap().query_map([],|row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,i64>(3)?))).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    keys.into_iter()
        .map(|(operation, idempotency, correlation, ordinal)| {
            decode_capsule(
                connection,
                audits,
                &operation,
                &idempotency,
                &correlation,
                ordinal,
            )
        })
        .collect()
}

#[derive(Debug)]
struct DbCommandResult {
    target_id: String,
    expected_version: Option<i64>,
    name: Option<String>,
    details: Option<String>,
    definition: Option<String>,
    owner: Option<String>,
    target: Option<String>,
    cadence: Option<String>,
    source: Option<String>,
    kpi_id: Option<String>,
    value: Option<String>,
    observed_at: Option<i64>,
    classification: Option<String>,
    provenance_kind: Option<String>,
    provenance_reference: Option<String>,
    result_kind: String,
    result_name: Option<String>,
    result_details: Option<String>,
    result_definition: Option<String>,
    result_owner: Option<String>,
    result_target: Option<String>,
    result_cadence: Option<String>,
    result_source: Option<String>,
    result_kpi_id: Option<String>,
    result_value: Option<String>,
    result_observed_at: Option<i64>,
    result_classification: String,
    result_provenance_kind: String,
    result_provenance_reference: Option<String>,
    result_version: i64,
    result_created_at: i64,
    result_updated_at: i64,
}

fn decode_capsule(
    connection: &Connection,
    audits: &HashMap<String, AuditEvent>,
    operation: &str,
    idempotency: &str,
    correlation: &str,
    ordinal: i64,
) -> pmc_domain::portfolio::PortfolioReplayCapsule {
    use pmc_domain::portfolio::{
        CommandIdentity, DerivedKpiObservationMutation, KpiDefinitionRecord, KpiObservationRecord,
        MutationOutcome, PortfolioPersistenceResult, PortfolioRecord, PortfolioReplayCapsule,
        ProductRecord, RoadmapRecord,
    };
    let row=connection.query_row("SELECT * FROM portfolio_command_results WHERE namespace='portfolio' AND operation=?1 AND idempotency_id=?2",params![operation,idempotency],|r| Ok(DbCommandResult {
        target_id:r.get("command_target_id")?,expected_version:r.get("command_expected_version")?,name:r.get("command_name")?,details:r.get("command_details")?,definition:r.get("command_definition")?,owner:r.get("command_owner")?,target:r.get("command_target")?,cadence:r.get("command_cadence")?,source:r.get("command_source")?,kpi_id:r.get("command_kpi_id")?,value:r.get("command_value")?,observed_at:r.get("command_observed_at")?,classification:r.get("command_classification")?,provenance_kind:r.get("command_provenance_kind")?,provenance_reference:r.get("command_provenance_reference")?,result_kind:r.get("result_kind")?,result_name:r.get("result_name")?,result_details:r.get("result_details")?,result_definition:r.get("result_definition")?,result_owner:r.get("result_owner")?,result_target:r.get("result_target")?,result_cadence:r.get("result_cadence")?,result_source:r.get("result_source")?,result_kpi_id:r.get("result_kpi_id")?,result_value:r.get("result_value")?,result_observed_at:r.get("result_observed_at")?,result_classification:r.get("result_classification")?,result_provenance_kind:r.get("result_provenance_kind")?,result_provenance_reference:r.get("result_provenance_reference")?,result_version:r.get("result_version")?,result_created_at:r.get("result_created_at")?,result_updated_at:r.get("result_updated_at")?
    })).unwrap();
    let class = row.classification.as_deref().map(classification);
    let create_provenance = || {
        decode_provenance(
            row.provenance_kind.as_deref().unwrap(),
            row.provenance_reference.clone(),
        )
    };
    let command = match operation {
        "create_portfolio" => CommandIdentity::CreatePortfolio {
            id: PortfolioId::parse(&row.target_id).unwrap(),
            name: short(row.name.as_deref().unwrap()),
            details: long(row.details.as_deref().unwrap()),
            classification: class,
            provenance: create_provenance(),
        },
        "update_portfolio" => CommandIdentity::UpdatePortfolio {
            id: PortfolioId::parse(&row.target_id).unwrap(),
            expected_version: version(row.expected_version.unwrap()),
            name: short(row.name.as_deref().unwrap()),
            details: long(row.details.as_deref().unwrap()),
            classification: class,
        },
        "create_product" => CommandIdentity::CreateProduct {
            id: ProductId::parse(&row.target_id).unwrap(),
            name: short(row.name.as_deref().unwrap()),
            details: long(row.details.as_deref().unwrap()),
            classification: class,
            provenance: create_provenance(),
        },
        "update_product" => CommandIdentity::UpdateProduct {
            id: ProductId::parse(&row.target_id).unwrap(),
            expected_version: version(row.expected_version.unwrap()),
            name: short(row.name.as_deref().unwrap()),
            details: long(row.details.as_deref().unwrap()),
            classification: class,
        },
        "create_roadmap" => CommandIdentity::CreateRoadmap {
            id: RoadmapId::parse(&row.target_id).unwrap(),
            name: short(row.name.as_deref().unwrap()),
            details: long(row.details.as_deref().unwrap()),
            classification: class,
            provenance: create_provenance(),
        },
        "update_roadmap" => CommandIdentity::UpdateRoadmap {
            id: RoadmapId::parse(&row.target_id).unwrap(),
            expected_version: version(row.expected_version.unwrap()),
            name: short(row.name.as_deref().unwrap()),
            details: long(row.details.as_deref().unwrap()),
            classification: class,
        },
        "create_kpi_definition" => CommandIdentity::CreateKpi {
            id: KpiId::parse(&row.target_id).unwrap(),
            name: short(row.name.as_deref().unwrap()),
            definition: long(row.definition.as_deref().unwrap()),
            owner: short(row.owner.as_deref().unwrap()),
            target: short(row.target.as_deref().unwrap()),
            cadence: short(row.cadence.as_deref().unwrap()),
            source: long(row.source.as_deref().unwrap()),
            classification: class,
            provenance: create_provenance(),
        },
        "update_kpi_definition" => CommandIdentity::UpdateKpi {
            id: KpiId::parse(&row.target_id).unwrap(),
            expected_version: version(row.expected_version.unwrap()),
            name: short(row.name.as_deref().unwrap()),
            definition: long(row.definition.as_deref().unwrap()),
            owner: short(row.owner.as_deref().unwrap()),
            target: short(row.target.as_deref().unwrap()),
            cadence: short(row.cadence.as_deref().unwrap()),
            source: long(row.source.as_deref().unwrap()),
            classification: class,
        },
        "create_kpi_observation" => CommandIdentity::CreateObservation {
            id: KpiObservationId::parse(&row.target_id).unwrap(),
            kpi_id: KpiId::parse(row.kpi_id.as_deref().unwrap()).unwrap(),
            value: short(row.value.as_deref().unwrap()),
            observed_at: UtcTimestamp::from_unix_millis(row.observed_at.unwrap()),
            source: long(row.source.as_deref().unwrap()),
            classification: class,
            provenance: create_provenance(),
        },
        "update_kpi_observation" => CommandIdentity::UpdateObservation {
            id: KpiObservationId::parse(&row.target_id).unwrap(),
            expected_version: version(row.expected_version.unwrap()),
            value: short(row.value.as_deref().unwrap()),
            observed_at: UtcTimestamp::from_unix_millis(row.observed_at.unwrap()),
            source: long(row.source.as_deref().unwrap()),
            classification: class,
        },
        _ => panic!("closed operation"),
    };
    let audit_rows=connection.prepare("SELECT ordinal,audit_event_id FROM portfolio_idempotency_outcome_audits WHERE namespace='portfolio' AND operation=?1 AND idempotency_id=?2 ORDER BY ordinal").unwrap().query_map(params![operation,idempotency],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?))).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    assert!(
        audit_rows
            .iter()
            .enumerate()
            .all(|(expected, (actual, _))| i64::try_from(expected).unwrap() == *actual),
        "audit ordinals must be contiguous"
    );
    let audit_ids = audit_rows.into_iter().map(|(_, id)| id).collect::<Vec<_>>();
    let audit = audits.get(&audit_ids[0]).unwrap().clone();
    let result_provenance = decode_provenance(
        &row.result_provenance_kind,
        row.result_provenance_reference.clone(),
    );
    let result_class = classification(&row.result_classification);
    let result_version = version(row.result_version);
    let result = match row.result_kind.as_str() {
        "portfolio" => PortfolioPersistenceResult::Portfolio {
            outcome: MutationOutcome {
                record: PortfolioRecord {
                    id: PortfolioId::parse(&row.target_id).unwrap(),
                    name: short(row.result_name.as_deref().unwrap()),
                    details: long(row.result_details.as_deref().unwrap()),
                    classification: result_class,
                    provenance: result_provenance,
                    version: result_version,
                },
                audit_event: audit,
            },
        },
        "product" => PortfolioPersistenceResult::Product {
            outcome: MutationOutcome {
                record: ProductRecord {
                    id: ProductId::parse(&row.target_id).unwrap(),
                    name: short(row.result_name.as_deref().unwrap()),
                    details: long(row.result_details.as_deref().unwrap()),
                    classification: result_class,
                    provenance: result_provenance,
                    version: result_version,
                },
                audit_event: audit,
            },
        },
        "roadmap" => PortfolioPersistenceResult::Roadmap {
            outcome: MutationOutcome {
                record: RoadmapRecord {
                    id: RoadmapId::parse(&row.target_id).unwrap(),
                    name: short(row.result_name.as_deref().unwrap()),
                    details: long(row.result_details.as_deref().unwrap()),
                    classification: result_class,
                    provenance: result_provenance,
                    version: result_version,
                },
                audit_event: audit,
            },
        },
        "kpi_definition" => PortfolioPersistenceResult::KpiDefinition {
            outcome: MutationOutcome {
                record: KpiDefinitionRecord {
                    id: KpiId::parse(&row.target_id).unwrap(),
                    name: short(row.result_name.as_deref().unwrap()),
                    definition: long(row.result_definition.as_deref().unwrap()),
                    owner: short(row.result_owner.as_deref().unwrap()),
                    target: short(row.result_target.as_deref().unwrap()),
                    cadence: short(row.result_cadence.as_deref().unwrap()),
                    source: long(row.result_source.as_deref().unwrap()),
                    classification: result_class,
                    provenance: result_provenance,
                    version: result_version,
                },
                audit_event: audit,
            },
        },
        "kpi_observation" => PortfolioPersistenceResult::KpiObservation {
            outcome: MutationOutcome {
                record: KpiObservationRecord {
                    id: KpiObservationId::parse(&row.target_id).unwrap(),
                    kpi_id: KpiId::parse(row.result_kpi_id.as_deref().unwrap()).unwrap(),
                    value: short(row.result_value.as_deref().unwrap()),
                    observed_at: UtcTimestamp::from_unix_millis(row.result_observed_at.unwrap()),
                    source: long(row.result_source.as_deref().unwrap()),
                    classification: result_class,
                    provenance: result_provenance,
                    version: result_version,
                    created_at: UtcTimestamp::from_unix_millis(row.result_created_at),
                    updated_at: UtcTimestamp::from_unix_millis(row.result_updated_at),
                },
                audit_event: audit,
            },
        },
        _ => panic!("closed result"),
    };
    let mutation_rows=connection.prepare("SELECT ordinal,observation_id,previous_version,resulting_version,previous_classification,resulting_classification,previous_updated_at,resulting_updated_at,audit_event_id FROM portfolio_derived_kpi_observation_mutations WHERE namespace='portfolio' AND operation=?1 AND idempotency_id=?2 ORDER BY ordinal").unwrap().query_map(params![operation,idempotency],|r| Ok((r.get::<_,i64>(0)?,DerivedKpiObservationMutation::new(KpiObservationId::parse(r.get::<_,String>(1)?).unwrap(),version(r.get(2)?),version(r.get(3)?),classification(&r.get::<_,String>(4)?),classification(&r.get::<_,String>(5)?),UtcTimestamp::from_unix_millis(r.get(6)?),UtcTimestamp::from_unix_millis(r.get(7)?),AuditEventId::parse(r.get::<_,String>(8)?).unwrap())))).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    assert!(
        mutation_rows
            .iter()
            .enumerate()
            .all(|(expected, (actual, _))| i64::try_from(expected).unwrap() == *actual),
        "mutation ordinals must be contiguous"
    );
    let mutations = mutation_rows
        .into_iter()
        .map(|(_, mutation)| mutation)
        .collect();
    PortfolioReplayCapsule::new(
        IdempotencyId::parse(idempotency).unwrap(),
        command,
        result,
        CorrelationId::parse(correlation).unwrap(),
        audit_ids
            .into_iter()
            .map(|v| AuditEventId::parse(v).unwrap())
            .collect(),
        u64::try_from(ordinal).unwrap(),
        mutations,
    )
}
