//! Typed SQLite persistence seam for the Relationship family's ordinary
//! (H1) commands: Stakeholder create/update. Link and H2b removal commands
//! land in later slices of the same rework.
//!
//! Unlike the Portfolio family (one wide `portfolio_command_results` table),
//! this schema is normalized per command shape
//! (`relationship_stakeholder_command_results`/`relationship_link_command_results`/
//! `relationship_h2b_command_results`), all anchored to one central
//! `relationship_replay_operations` table with a globally contiguous
//! `operation_ordinal` across every Relationship-family command -- closer in
//! shape to Risk/Issue H2a's replay tables than to Portfolio's.
//!
//! Relationship audits always use `AuditModule::Portfolio` and the fixed
//! effect code `relationship.authoritative-record-changed`, matching
//! `relationships.rs::append_audit` exactly.

use pmc_domain::{
    audit::{
        AuditAction, AuditActor, AuditDisposition, AuditEffectCode, AuditEffectScope, AuditEvent,
        AuditEventCode, AuditExecutionOutcome, AuditModule, AuditPolicyOutcome, AuditTarget,
    },
    classification::DataClassification,
    error::{DomainError, ErrorCode, MessageKey},
    identity::{
        AggregateVersion, AuditEventId, InitiativeId, KpiId, PortfolioId, ProductId, ProjectId,
        RelationshipId, RoadmapId, StakeholderId,
    },
    provenance::{Provenance, ProvenanceReference},
    relationships::{
        CreateStakeholder, EndpointSnapshot, InitiativeSnapshot, KpiSnapshot,
        LinkInitiativeProject, LinkPortfolioInitiative, LinkPortfolioProduct, LinkProductKpi,
        LinkProductRoadmap, LinkProjectProduct, LinkStakeholderRelationship, MutationOutcome,
        OperationContext, PortfolioSnapshot, ProductSnapshot, ProjectSnapshot, RelationshipKind,
        RelationshipPersistenceRecord, RelationshipRecord, RoadmapSnapshot, StakeholderKind,
        StakeholderPersistenceRecord, StakeholderRecord, StakeholderSubject,
        UpdateStakeholderDetails,
    },
    time::UtcTimestamp,
};
use rusqlite::{OptionalExtension, Transaction};

use super::{LedgerTransactionError, SqliteProductLedger};

/// The single constant effect code every Relationship-family mutation uses,
/// matching `relationships.rs::append_audit`'s own constant.
pub(super) const RELATIONSHIP_EFFECT_CODE: &str = "relationship.authoritative-record-changed";

impl SqliteProductLedger {
    pub fn create_stakeholder(
        &mut self,
        command: CreateStakeholder,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<StakeholderRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some(existing) = tx
                .query_row(
                    "SELECT command_id,command_name,command_stakeholder_kind,command_classification,command_provenance_kind,command_provenance_reference FROM relationship_stakeholder_command_results WHERE idempotency_id=?1 AND command_kind='create_stakeholder'",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, Option<String>>(5)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                let matches = existing.0 == command.id.as_str()
                    && existing.1 == command.name.as_str()
                    && existing.2 == command.kind.as_persisted()
                    && existing.3.as_deref() == command.classification.map(DataClassification::as_persisted)
                    && existing.4 == command.provenance.kind_persisted()
                    && existing.5.as_deref() == command.provenance.reference().map(ProvenanceReference::as_str);
                if matches {
                    return decode_stakeholder_outcome(tx, &context);
                }
                return Err(idempotency_conflict(&context));
            }
            let exists: i64 = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM aggregate_registry WHERE id=?1)",
                    [command.id.as_str()],
                    |row| row.get(0),
                )
                .map_err(|_| storage_error(&context))?;
            if exists != 0 {
                return Err(domain_conflict(&context));
            }
            let classification = command.classification.unwrap_or_default();
            let occurred_millis = occurred_at.unix_millis();
            tx.execute(
                "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES(?1,'stakeholder',1,?2,?3,?3)",
                rusqlite::params![command.id.as_str(), classification.as_persisted(), occurred_millis],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO stakeholders(id,name,kind,provenance_kind,provenance_reference) VALUES(?1,?2,?3,?4,?5)",
                rusqlite::params![
                    command.id.as_str(),
                    command.name.as_str(),
                    command.kind.as_persisted(),
                    command.provenance.kind_persisted(),
                    command.provenance.reference().map(ProvenanceReference::as_str),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let audit = build_audit(
                audit_event_id,
                occurred_at,
                AuditTarget::Stakeholder(command.id.clone()),
                "relationship.stakeholder.created",
                &context,
            )?;
            persist_relationship_audit(tx, &audit, "stakeholder", command.id.as_str(), &context)?;
            let ordinal = next_relationship_operation_ordinal(tx, &context)?;
            tx.execute(
                "INSERT INTO relationship_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES(?1,'create_stakeholder',?2,?3,'stakeholder',?4)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    ordinal,
                    command.id.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO relationship_replay_audits(idempotency_id,ordinal,audit_event_id,correlation_id) VALUES(?1,0,?2,?3)",
                rusqlite::params![context.idempotency_id.as_str(), audit.id().as_str(), context.correlation_id.as_str()],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO relationship_stakeholder_command_results (idempotency_id,command_kind,command_id,result_id,command_name,command_stakeholder_kind,command_classification,command_provenance_kind,command_provenance_reference,result_name,result_stakeholder_kind,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at,effect_scope) VALUES (?1,'create_stakeholder',?2,?2,?3,?4,?5,?6,?7,?3,?4,?8,?6,?7,1,?9,?9,'complete')",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.id.as_str(),
                    command.name.as_str(),
                    command.kind.as_persisted(),
                    command.classification.map(DataClassification::as_persisted),
                    command.provenance.kind_persisted(),
                    command.provenance.reference().map(ProvenanceReference::as_str),
                    classification.as_persisted(),
                    occurred_millis,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx
                .execute(
                    "UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2",
                    rusqlite::params![expected_revision + 1, expected_revision],
                )
                .map_err(|_| storage_error(&context))?
                != 1
            {
                return Err(storage_error(&context));
            }
            let record = StakeholderRecord::rehydrate(StakeholderPersistenceRecord {
                id: command.id,
                name: command.name,
                kind: command.kind,
                classification,
                provenance: command.provenance,
                version: AggregateVersion::initial(),
                created_at: occurred_at,
                updated_at: occurred_at,
            })
            .map_err(|_| storage_error(&context))?;
            Ok(MutationOutcome::from_persistence(
                record,
                vec![audit.id().clone()],
                AuditEffectScope::Complete,
            ))
        })
    }

    /// `next_audit_event_id` is called once for the Stakeholder's own audit
    /// and once more per Relationship the classification-reclassification
    /// cascade actually touches -- the count is only known once affected
    /// relationships are found, matching
    /// `portfolio_repository.rs::update_kpi_definition_details`'s use of the
    /// same closure shape for the same reason.
    pub fn update_stakeholder(
        &mut self,
        command: UpdateStakeholderDetails,
        mut next_audit_event_id: impl FnMut() -> AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<StakeholderRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some(existing) = tx
                .query_row(
                    "SELECT command_id,command_expected_version,command_name,command_classification FROM relationship_stakeholder_command_results WHERE idempotency_id=?1 AND command_kind='update_stakeholder'",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, Option<String>>(3)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                let matches = existing.0 == command.id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == command.name.as_str()
                    && existing.3.as_deref() == command.classification.map(DataClassification::as_persisted);
                if matches {
                    return decode_stakeholder_outcome(tx, &context);
                }
                return Err(idempotency_conflict(&context));
            }
            let (current_kind, current_classification, current_version, current_created_at): (
                String,
                String,
                i64,
                i64,
            ) = tx
                .query_row(
                    "SELECT stakeholders.kind,registry.classification,registry.version,registry.created_at FROM stakeholders JOIN aggregate_registry registry ON registry.id=stakeholders.id AND registry.aggregate_type='stakeholder' WHERE stakeholders.id=?1",
                    [command.id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| domain_not_found(&context))?;
            if current_version != i64::try_from(command.expected_version.get()).unwrap_or(-1) {
                return Err(domain_conflict(&context));
            }
            let current_classification = DataClassification::from_persisted(&current_classification)
                .map_err(|_| storage_error(&context))?;
            let classification = match command.classification {
                Some(requested) => {
                    if current_classification.combine(requested) != requested {
                        return Err(policy_denied(&context));
                    }
                    requested
                }
                None => current_classification,
            };
            let next_version = command
                .expected_version
                .next()
                .ok_or_else(|| domain_conflict(&context))?;
            let occurred_millis = occurred_at.unix_millis();
            tx.execute(
                "UPDATE stakeholders SET name=?1 WHERE id=?2",
                rusqlite::params![command.name.as_str(), command.id.as_str()],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "UPDATE aggregate_registry SET version=?1,classification=?2,updated_at=?3 WHERE id=?4 AND aggregate_type='stakeholder'",
                rusqlite::params![
                    i64::try_from(next_version.get()).map_err(|_| storage_error(&context))?,
                    classification.as_persisted(),
                    occurred_millis,
                    command.id.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let audit = build_audit(
                next_audit_event_id(),
                occurred_at,
                AuditTarget::Stakeholder(command.id.clone()),
                "relationship.stakeholder.updated",
                &context,
            )?;
            persist_relationship_audit(tx, &audit, "stakeholder", command.id.as_str(), &context)?;

            // relationship_replay_audits' FK to relationship_replay_operations
            // is NOT deferrable, so the operations row (and the main audit's
            // own ordinal-0 audits row) must exist before the cascade below
            // inserts its own ordinal 1..N audits rows.
            let ordinal = next_relationship_operation_ordinal(tx, &context)?;
            tx.execute(
                "INSERT INTO relationship_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES(?1,'update_stakeholder',?2,?3,'stakeholder',?4)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    ordinal,
                    command.id.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO relationship_replay_audits(idempotency_id,ordinal,audit_event_id,correlation_id) VALUES(?1,0,?2,?3)",
                rusqlite::params![context.idempotency_id.as_str(), audit.id().as_str(), context.correlation_id.as_str()],
            )
            .map_err(|_| storage_error(&context))?;

            // Cascade: every Relationship endpoint snapshot for this
            // Stakeholder is frozen at link/update time, not dynamically
            // re-read (relationships.rs::update_stakeholder). A classification
            // change re-derives every relationship this Stakeholder is an
            // endpoint of, in relationship-id order, matching the domain
            // exactly.
            let mut audit_ids = vec![audit.id().clone()];
            let mut derived_ordinal: i64 = 1;
            if classification != current_classification {
                let affected: Vec<(String, i64)> = {
                    let mut statement = tx
                        .prepare(
                            "SELECT DISTINCT relationship_endpoints.relationship_id,relationships_registry.version FROM relationship_endpoints JOIN aggregate_registry relationships_registry ON relationships_registry.id=relationship_endpoints.relationship_id AND relationships_registry.aggregate_type='relationship' WHERE relationship_endpoints.target_type='stakeholder' AND relationship_endpoints.target_id=?1 ORDER BY relationship_endpoints.relationship_id",
                        )
                        .map_err(|_| storage_error(&context))?;
                    let rows = statement
                        .query_map([command.id.as_str()], |row| Ok((row.get(0)?, row.get(1)?)))
                        .map_err(|_| storage_error(&context))?
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|_| storage_error(&context))?;
                    rows
                };
                for (relationship_id, relationship_version) in affected {
                    tx.execute(
                        "UPDATE relationship_endpoints SET target_version=?1,target_classification=?2 WHERE relationship_id=?3 AND target_type='stakeholder' AND target_id=?4",
                        rusqlite::params![
                            i64::try_from(next_version.get()).map_err(|_| storage_error(&context))?,
                            classification.as_persisted(),
                            relationship_id.as_str(),
                            command.id.as_str(),
                        ],
                    )
                    .map_err(|_| storage_error(&context))?;
                    let endpoint_classifications: Vec<String> = {
                        let mut statement = tx
                            .prepare("SELECT target_classification FROM relationship_endpoints WHERE relationship_id=?1 ORDER BY ordinal")
                            .map_err(|_| storage_error(&context))?;
                        let rows = statement
                            .query_map([relationship_id.as_str()], |row| row.get(0))
                            .map_err(|_| storage_error(&context))?
                            .collect::<Result<Vec<_>, _>>()
                            .map_err(|_| storage_error(&context))?;
                        rows
                    };
                    let mut derived_classification = DataClassification::Public;
                    for value in &endpoint_classifications {
                        let value = DataClassification::from_persisted(value)
                            .map_err(|_| storage_error(&context))?;
                        derived_classification = derived_classification.combine(value);
                    }
                    let derived_version = relationship_version + 1;
                    tx.execute(
                        "UPDATE aggregate_registry SET version=?1,classification=?2,updated_at=?3 WHERE id=?4 AND aggregate_type='relationship'",
                        rusqlite::params![
                            derived_version,
                            derived_classification.as_persisted(),
                            occurred_millis,
                            relationship_id.as_str(),
                        ],
                    )
                    .map_err(|_| storage_error(&context))?;
                    let derived_audit = build_audit(
                        next_audit_event_id(),
                        occurred_at,
                        AuditTarget::Stakeholder(command.id.clone()),
                        "relationship.stakeholder_relationship.reclassified",
                        &context,
                    )?;
                    persist_relationship_audit(tx, &derived_audit, "relationship", relationship_id.as_str(), &context)?;
                    tx.execute(
                        "INSERT INTO relationship_replay_audits(idempotency_id,ordinal,audit_event_id,correlation_id) VALUES(?1,?2,?3,?4)",
                        rusqlite::params![
                            context.idempotency_id.as_str(),
                            derived_ordinal,
                            derived_audit.id().as_str(),
                            context.correlation_id.as_str(),
                        ],
                    )
                    .map_err(|_| storage_error(&context))?;
                    audit_ids.push(derived_audit.id().clone());
                    derived_ordinal += 1;
                }
            }

            tx.execute(
                "INSERT INTO relationship_stakeholder_command_results (idempotency_id,command_kind,command_id,result_id,command_expected_version,command_name,command_classification,result_name,result_stakeholder_kind,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at,effect_scope) VALUES (?1,'update_stakeholder',?2,?2,?3,?4,?5,?4,?6,?7,(SELECT provenance_kind FROM stakeholders WHERE id=?2),(SELECT provenance_reference FROM stakeholders WHERE id=?2),?8,?9,?10,'complete')",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                    command.name.as_str(),
                    command.classification.map(DataClassification::as_persisted),
                    current_kind.as_str(),
                    classification.as_persisted(),
                    i64::try_from(next_version.get()).map_err(|_| storage_error(&context))?,
                    current_created_at,
                    occurred_millis,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx
                .execute(
                    "UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2",
                    rusqlite::params![expected_revision + 1, expected_revision],
                )
                .map_err(|_| storage_error(&context))?
                != 1
            {
                return Err(storage_error(&context));
            }
            let (provenance_kind, provenance_reference): (String, Option<String>) = tx
                .query_row(
                    "SELECT provenance_kind,provenance_reference FROM stakeholders WHERE id=?1",
                    [command.id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|_| storage_error(&context))?;
            let provenance = decode_provenance(&provenance_kind, provenance_reference, &context)?;
            let kind = StakeholderKind::from_persisted(&current_kind).map_err(|_| storage_error(&context))?;
            let record = StakeholderRecord::rehydrate(StakeholderPersistenceRecord {
                id: command.id,
                name: command.name,
                kind,
                classification,
                provenance,
                version: next_version,
                created_at: UtcTimestamp::from_unix_millis(current_created_at),
                updated_at: occurred_at,
            })
            .map_err(|_| storage_error(&context))?;
            Ok(MutationOutcome::from_persistence(
                record,
                audit_ids,
                AuditEffectScope::Complete,
            ))
        })
    }

    pub fn link_portfolio_product(
        &mut self,
        command: LinkPortfolioProduct,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<RelationshipRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            link_ordinary(
                &mut transaction.transaction,
                expected_revision,
                command.id,
                RelationshipKind::PortfolioProduct,
                [
                    EndpointRequest::new(
                        "portfolio",
                        "portfolio",
                        command.portfolio_id.as_str(),
                        command.expected_portfolio_version.get(),
                    ),
                    EndpointRequest::new(
                        "product",
                        "product",
                        command.product_id.as_str(),
                        command.expected_product_version.get(),
                    ),
                ],
                None,
                &context,
                audit_event_id,
                occurred_at,
            )
        })
    }

    pub fn link_portfolio_initiative(
        &mut self,
        command: LinkPortfolioInitiative,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<RelationshipRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            link_ordinary(
                &mut transaction.transaction,
                expected_revision,
                command.id,
                RelationshipKind::PortfolioInitiative,
                [
                    EndpointRequest::new(
                        "portfolio",
                        "portfolio",
                        command.portfolio_id.as_str(),
                        command.expected_portfolio_version.get(),
                    ),
                    EndpointRequest::new(
                        "initiative",
                        "initiative",
                        command.initiative_id.as_str(),
                        command.expected_initiative_version.get(),
                    ),
                ],
                None,
                &context,
                audit_event_id,
                occurred_at,
            )
        })
    }

    pub fn link_product_roadmap(
        &mut self,
        command: LinkProductRoadmap,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<RelationshipRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            link_ordinary(
                &mut transaction.transaction,
                expected_revision,
                command.id,
                RelationshipKind::ProductRoadmap,
                [
                    EndpointRequest::new(
                        "product",
                        "product",
                        command.product_id.as_str(),
                        command.expected_product_version.get(),
                    ),
                    EndpointRequest::new(
                        "roadmap",
                        "roadmap",
                        command.roadmap_id.as_str(),
                        command.expected_roadmap_version.get(),
                    ),
                ],
                None,
                &context,
                audit_event_id,
                occurred_at,
            )
        })
    }

    pub fn link_product_kpi(
        &mut self,
        command: LinkProductKpi,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<RelationshipRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            link_ordinary(
                &mut transaction.transaction,
                expected_revision,
                command.id,
                RelationshipKind::ProductKpi,
                [
                    EndpointRequest::new(
                        "product",
                        "product",
                        command.product_id.as_str(),
                        command.expected_product_version.get(),
                    ),
                    EndpointRequest::new(
                        "kpi_definition",
                        "kpi",
                        command.kpi_id.as_str(),
                        command.expected_kpi_version.get(),
                    ),
                ],
                None,
                &context,
                audit_event_id,
                occurred_at,
            )
        })
    }

    pub fn link_initiative_project(
        &mut self,
        command: LinkInitiativeProject,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<RelationshipRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            link_ordinary(
                &mut transaction.transaction,
                expected_revision,
                command.id,
                RelationshipKind::InitiativeProject,
                [
                    EndpointRequest::new(
                        "initiative",
                        "initiative",
                        command.initiative_id.as_str(),
                        command.expected_initiative_version.get(),
                    ),
                    EndpointRequest::new(
                        "project",
                        "project",
                        command.project_id.as_str(),
                        command.expected_project_version.get(),
                    ),
                ],
                None,
                &context,
                audit_event_id,
                occurred_at,
            )
        })
    }

    pub fn link_project_product(
        &mut self,
        command: LinkProjectProduct,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<RelationshipRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            link_ordinary(
                &mut transaction.transaction,
                expected_revision,
                command.id,
                RelationshipKind::ProjectProduct,
                [
                    EndpointRequest::new(
                        "project",
                        "project",
                        command.project_id.as_str(),
                        command.expected_project_version.get(),
                    ),
                    EndpointRequest::new(
                        "product",
                        "product",
                        command.product_id.as_str(),
                        command.expected_product_version.get(),
                    ),
                ],
                None,
                &context,
                audit_event_id,
                occurred_at,
            )
        })
    }

    /// Milestone subjects are not yet supported: Milestone itself has no
    /// real SQLite writer (Delivery/#54, a separate not-yet-started slice),
    /// so there is no durable Milestone record to link against yet. Every
    /// other `StakeholderSubject` kind is fully supported.
    pub fn link_stakeholder_relationship(
        &mut self,
        command: LinkStakeholderRelationship,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<RelationshipRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        let subject = match &command.subject {
            StakeholderSubject::Portfolio(id) => EndpointRequest::new(
                "portfolio",
                "portfolio",
                id.as_str(),
                command.expected_subject_version.get(),
            ),
            StakeholderSubject::Product(id) => EndpointRequest::new(
                "product",
                "product",
                id.as_str(),
                command.expected_subject_version.get(),
            ),
            StakeholderSubject::Initiative(id) => EndpointRequest::new(
                "initiative",
                "initiative",
                id.as_str(),
                command.expected_subject_version.get(),
            ),
            StakeholderSubject::Project(id) => EndpointRequest::new(
                "project",
                "project",
                id.as_str(),
                command.expected_subject_version.get(),
            ),
            StakeholderSubject::Roadmap(id) => EndpointRequest::new(
                "roadmap",
                "roadmap",
                id.as_str(),
                command.expected_subject_version.get(),
            ),
            StakeholderSubject::Kpi(id) => EndpointRequest::new(
                "kpi_definition",
                "kpi",
                id.as_str(),
                command.expected_subject_version.get(),
            ),
            // A Milestone subject is not implemented in this writer. Say so
            // as a validation refusal, not as a retryable storage failure.
            StakeholderSubject::Milestone(_) => {
                return Err(LedgerTransactionError::Operation(
                    milestone_subject_not_supported(&context),
                ));
            }
        };
        self.with_immediate_transaction(|transaction| {
            link_ordinary(
                &mut transaction.transaction,
                expected_revision,
                command.id,
                RelationshipKind::StakeholderSubject,
                [
                    EndpointRequest::new(
                        "stakeholder",
                        "stakeholder",
                        command.stakeholder_id.as_str(),
                        command.expected_stakeholder_version.get(),
                    ),
                    subject,
                ],
                Some(command.purpose),
                &context,
                audit_event_id,
                occurred_at,
            )
        })
    }
}

/// One requested Link endpoint: the aggregate-registry-form type (used for
/// `aggregate_registry`/`relationship_endpoints`, which follow
/// `aggregate_registry.aggregate_type` naming -- notably `kpi_definition`,
/// not `kpi`) paired with the shorter `link_type` used by
/// `relationship_link_command_endpoints`/`relationship_link_result_endpoints`
/// (which use `kpi`). The two schema families use different vocabularies for
/// the same KPI Definition endpoint kind; this is not a typo.
struct EndpointRequest {
    aggregate_type: &'static str,
    link_type: &'static str,
    id: String,
    expected_version: u64,
    /// Milestone endpoints alone carry a parent Project id, matching
    /// `relationship_endpoints`'s `CHECK((target_type='milestone')=(parent_project_id IS NOT NULL))`.
    parent_project_id: Option<String>,
}
impl EndpointRequest {
    fn new(
        aggregate_type: &'static str,
        link_type: &'static str,
        id: &str,
        expected_version: u64,
    ) -> Self {
        Self {
            aggregate_type,
            link_type,
            id: id.to_owned(),
            expected_version,
            parent_project_id: None,
        }
    }
}

#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn link_ordinary(
    tx: &mut Transaction<'_>,
    expected_revision: u64,
    relationship_id: RelationshipId,
    kind: RelationshipKind,
    endpoints: [EndpointRequest; 2],
    purpose: Option<pmc_domain::relationships::StakeholderRelationshipPurpose>,
    context: &OperationContext,
    audit_event_id: AuditEventId,
    occurred_at: UtcTimestamp,
) -> Result<MutationOutcome<RelationshipRecord>, DomainError> {
    let purpose_persisted = purpose.map(|value| value.as_persisted());
    let tombstoned: i64 = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM relationship_replay_tombstones WHERE idempotency_id=?1)",
            [context.idempotency_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    if tombstoned != 0 {
        return Err(idempotency_conflict(context));
    }
    if let Some((existing, stored_purpose)) = tx
        .query_row(
            "SELECT relationship_id,purpose FROM relationship_link_command_results WHERE idempotency_id=?1",
            [context.idempotency_id.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()
        .map_err(|_| storage_error(context))?
    {
        let stored_endpoints: Vec<(String, String, i64)> = {
            let mut statement = tx
                .prepare("SELECT endpoint_type,endpoint_id,expected_version FROM relationship_link_command_endpoints WHERE idempotency_id=?1 ORDER BY ordinal")
                .map_err(|_| storage_error(context))?;
            let rows = statement
                .query_map([context.idempotency_id.as_str()], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })
                .map_err(|_| storage_error(context))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| storage_error(context))?;
            rows
        };
        let matches = existing == relationship_id.as_str()
            && stored_purpose.as_deref() == purpose_persisted
            && stored_endpoints.len() == 2
            && stored_endpoints.iter().zip(&endpoints).all(
                |((stored_type, stored_id, stored_version), request)| {
                    stored_type == request.link_type
                        && stored_id == &request.id
                        && *stored_version == i64::try_from(request.expected_version).unwrap_or(-1)
                },
            );
        if matches {
            return decode_relationship_outcome(tx, context);
        }
        return Err(idempotency_conflict(context));
    }

    let mut resolved = Vec::with_capacity(2);
    for request in &endpoints {
        let (current_version, current_classification): (i64, String) = tx
            .query_row(
                "SELECT version,classification FROM aggregate_registry WHERE id=?1 AND aggregate_type=?2",
                [request.id.as_str(), request.aggregate_type],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|_| storage_error(context))?
            .ok_or_else(|| domain_not_found(context))?;
        if current_version != i64::try_from(request.expected_version).unwrap_or(-1) {
            return Err(domain_conflict(context));
        }
        let classification = DataClassification::from_persisted(&current_classification)
            .map_err(|_| storage_error(context))?;
        if classification == DataClassification::Unclassified {
            return Err(policy_denied(context));
        }
        let version = AggregateVersion::new(
            u64::try_from(current_version).map_err(|_| storage_error(context))?,
        )
        .map_err(|_| storage_error(context))?;
        resolved.push(build_endpoint_snapshot(
            request.aggregate_type,
            &request.id,
            version,
            classification,
            context,
        )?);
    }

    let existing_semantic: Option<String> = tx
        .query_row(
            "SELECT r.id FROM relationships r JOIN relationship_endpoints e0 ON e0.relationship_id=r.id AND e0.ordinal=0 AND e0.target_type=?1 AND e0.target_id=?2 JOIN relationship_endpoints e1 ON e1.relationship_id=r.id AND e1.ordinal=1 AND e1.target_type=?3 AND e1.target_id=?4 WHERE r.kind=?5 AND r.purpose IS ?6",
            rusqlite::params![
                endpoints[0].aggregate_type,
                endpoints[0].id,
                endpoints[1].aggregate_type,
                endpoints[1].id,
                kind.as_persisted(),
                purpose_persisted,
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| storage_error(context))?;

    let occurred_millis = occurred_at.unix_millis();
    let ordinal = next_relationship_operation_ordinal(tx, context)?;

    if let Some(existing_id) = existing_semantic {
        // Relinking an already-linked pair must reflect each endpoint's
        // *current* state, not whatever was true when it was first linked --
        // matches `relationships.rs::current_endpoints`, which always
        // re-resolves live (via the resolver for every kind, or the
        // service's own always-current Stakeholder cache) rather than
        // trusting a stored snapshot. `relationship_endpoints`' own
        // target_version/target_classification columns are only accurate as
        // of link time; nothing updates them afterward except the Stakeholder
        // cascade in `update_stakeholder`, so reading them here for a
        // Portfolio/Product/etc. endpoint that changed since linking would
        // silently return stale data.
        let endpoint_identities: Vec<(String, String)> = {
            let mut statement = tx
                .prepare("SELECT target_type,target_id FROM relationship_endpoints WHERE relationship_id=?1 ORDER BY ordinal")
                .map_err(|_| storage_error(context))?;
            let rows = statement
                .query_map([existing_id.as_str()], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(|_| storage_error(context))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| storage_error(context))?;
            rows
        };
        let mut current: Vec<(String, String, i64, String)> =
            Vec::with_capacity(endpoint_identities.len());
        for (target_type, target_id) in endpoint_identities {
            let (live_version, live_classification): (i64, String) = tx
                .query_row(
                    "SELECT version,classification FROM aggregate_registry WHERE id=?1 AND aggregate_type=?2",
                    [target_id.as_str(), target_type.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|_| storage_error(context))?;
            current.push((target_type, target_id, live_version, live_classification));
        }
        let mut derived_classification = DataClassification::Public;
        for (_, _, _, classification) in &current {
            let value = DataClassification::from_persisted(classification)
                .map_err(|_| storage_error(context))?;
            derived_classification = derived_classification.combine(value);
        }
        let (result_version, result_created_at, result_updated_at): (i64, i64, i64) = tx
            .query_row(
                "SELECT version,created_at,updated_at FROM aggregate_registry WHERE id=?1",
                [existing_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| storage_error(context))?;
        tx.execute(
            "INSERT INTO relationship_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES(?1,'link',?2,?3,'relationship',?4)",
            rusqlite::params![context.idempotency_id.as_str(), context.correlation_id.as_str(), ordinal, existing_id.as_str()],
        )
        .map_err(|_| storage_error(context))?;
        tx.execute(
            "INSERT INTO relationship_link_command_results(idempotency_id,relationship_id,relationship_kind,purpose,result_version,result_classification,result_created_at,result_updated_at,effect_scope) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'none')",
            rusqlite::params![
                context.idempotency_id.as_str(),
                existing_id.as_str(),
                kind.as_persisted(),
                purpose_persisted,
                result_version,
                derived_classification.as_persisted(),
                result_created_at,
                result_updated_at,
            ],
        )
        .map_err(|_| storage_error(context))?;
        for (ordinal_index, request) in endpoints.iter().enumerate() {
            let ordinal_index = i64::try_from(ordinal_index).map_err(|_| storage_error(context))?;
            let snapshot_version =
                i64::try_from(endpoint_snapshot_version(&resolved[ordinal_index as usize]).get())
                    .map_err(|_| storage_error(context))?;
            tx.execute(
                "INSERT INTO relationship_link_command_endpoints(idempotency_id,ordinal,endpoint_type,endpoint_id,expected_version,snapshot_version,classification) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    ordinal_index,
                    request.link_type,
                    request.id,
                    i64::try_from(request.expected_version).map_err(|_| storage_error(context))?,
                    snapshot_version,
                    endpoint_snapshot_classification(&resolved[ordinal_index as usize]).as_persisted(),
                ],
            )
            .map_err(|_| storage_error(context))?;
        }
        for (ordinal_index, (target_type, target_id, target_version, target_classification)) in
            current.iter().enumerate()
        {
            let ordinal_index = i64::try_from(ordinal_index).map_err(|_| storage_error(context))?;
            let link_type = aggregate_type_to_link_type(target_type);
            tx.execute(
                "INSERT INTO relationship_link_result_endpoints(idempotency_id,ordinal,endpoint_type,endpoint_id,endpoint_version,classification) VALUES(?1,?2,?3,?4,?5,?6)",
                rusqlite::params![context.idempotency_id.as_str(), ordinal_index, link_type, target_id, target_version, target_classification],
            )
            .map_err(|_| storage_error(context))?;
        }
        let expected_revision =
            i64::try_from(expected_revision).map_err(|_| storage_error(context))?;
        if tx
            .execute(
                "UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2",
                rusqlite::params![expected_revision + 1, expected_revision],
            )
            .map_err(|_| storage_error(context))?
            != 1
        {
            return Err(storage_error(context));
        }
        let mut normalized_endpoints = Vec::with_capacity(current.len());
        for (target_type, target_id, target_version, target_classification) in &current {
            let version = AggregateVersion::new(
                u64::try_from(*target_version).map_err(|_| storage_error(context))?,
            )
            .map_err(|_| storage_error(context))?;
            let classification = DataClassification::from_persisted(target_classification)
                .map_err(|_| storage_error(context))?;
            normalized_endpoints.push(build_endpoint_snapshot(
                target_type,
                target_id,
                version,
                classification,
                context,
            )?);
        }
        let record = RelationshipRecord::rehydrate(RelationshipPersistenceRecord {
            id: RelationshipId::parse(existing_id).map_err(|_| storage_error(context))?,
            kind,
            endpoints: normalized_endpoints,
            purpose,
            classification: derived_classification,
            version: AggregateVersion::new(
                u64::try_from(result_version).map_err(|_| storage_error(context))?,
            )
            .map_err(|_| storage_error(context))?,
            created_at: UtcTimestamp::from_unix_millis(result_created_at),
            updated_at: UtcTimestamp::from_unix_millis(result_updated_at),
        })
        .map_err(|_| storage_error(context))?;
        return Ok(MutationOutcome::from_persistence(
            record,
            Vec::new(),
            AuditEffectScope::None,
        ));
    }

    // Relationship IDs are authority identities, not recyclable row keys
    // (matches `relationships.rs::link`'s own comment to the same effect):
    // once a relationship ID has ever been linked, it can never be reused
    // for a brand-new link, even after H2b removal deletes it from
    // `aggregate_registry`. `relationship_link_command_results` rows are
    // never deleted, so checking history there (not just current existence)
    // is what actually enforces this.
    let exists: i64 = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM aggregate_registry WHERE id=?1) OR EXISTS(SELECT 1 FROM relationship_link_command_results WHERE relationship_id=?1)",
            [relationship_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    if exists != 0 {
        return Err(domain_conflict(context));
    }
    let classification = resolved.iter().fold(DataClassification::Public, |a, v| {
        a.combine(endpoint_snapshot_classification(v))
    });
    tx.execute(
        "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES(?1,'relationship',1,?2,?3,?3)",
        rusqlite::params![relationship_id.as_str(), classification.as_persisted(), occurred_millis],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO relationships(id,kind,purpose) VALUES(?1,?2,?3)",
        rusqlite::params![
            relationship_id.as_str(),
            kind.as_persisted(),
            purpose_persisted
        ],
    )
    .map_err(|_| storage_error(context))?;
    for (ordinal_index, request) in endpoints.iter().enumerate() {
        let ordinal_index_i64 = i64::try_from(ordinal_index).map_err(|_| storage_error(context))?;
        let snapshot_version =
            i64::try_from(endpoint_snapshot_version(&resolved[ordinal_index]).get())
                .map_err(|_| storage_error(context))?;
        tx.execute(
            "INSERT INTO relationship_endpoints(relationship_id,ordinal,target_type,target_id,target_version,target_classification,parent_project_id) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            rusqlite::params![
                relationship_id.as_str(),
                ordinal_index_i64,
                request.aggregate_type,
                request.id,
                snapshot_version,
                endpoint_snapshot_classification(&resolved[ordinal_index]).as_persisted(),
                request.parent_project_id,
            ],
        )
        .map_err(|_| storage_error(context))?;
    }
    let audit = build_audit(
        audit_event_id,
        occurred_at,
        endpoint_snapshot_audit_target(&resolved[0]),
        relationship_code(kind),
        context,
    )?;
    persist_relationship_audit(
        tx,
        &audit,
        "relationship",
        relationship_id.as_str(),
        context,
    )?;
    tx.execute(
        "INSERT INTO relationship_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES(?1,'link',?2,?3,'relationship',?4)",
        rusqlite::params![context.idempotency_id.as_str(), context.correlation_id.as_str(), ordinal, relationship_id.as_str()],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO relationship_replay_audits(idempotency_id,ordinal,audit_event_id,correlation_id) VALUES(?1,0,?2,?3)",
        rusqlite::params![context.idempotency_id.as_str(), audit.id().as_str(), context.correlation_id.as_str()],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO relationship_link_command_results(idempotency_id,relationship_id,relationship_kind,purpose,result_version,result_classification,result_created_at,result_updated_at,effect_scope) VALUES(?1,?2,?3,?4,1,?5,?6,?6,'complete')",
        rusqlite::params![
            context.idempotency_id.as_str(),
            relationship_id.as_str(),
            kind.as_persisted(),
            purpose_persisted,
            classification.as_persisted(),
            occurred_millis,
        ],
    )
    .map_err(|_| storage_error(context))?;
    for (ordinal_index, request) in endpoints.iter().enumerate() {
        let ordinal_index_i64 = i64::try_from(ordinal_index).map_err(|_| storage_error(context))?;
        let expected_version =
            i64::try_from(request.expected_version).map_err(|_| storage_error(context))?;
        let classification =
            endpoint_snapshot_classification(&resolved[ordinal_index]).as_persisted();
        tx.execute(
            "INSERT INTO relationship_link_command_endpoints(idempotency_id,ordinal,endpoint_type,endpoint_id,expected_version,snapshot_version,classification) VALUES(?1,?2,?3,?4,?5,?5,?6)",
            rusqlite::params![
                context.idempotency_id.as_str(),
                ordinal_index_i64,
                request.link_type,
                request.id,
                expected_version,
                classification,
            ],
        )
        .map_err(|_| storage_error(context))?;
        tx.execute(
            "INSERT INTO relationship_link_result_endpoints(idempotency_id,ordinal,endpoint_type,endpoint_id,endpoint_version,classification) VALUES(?1,?2,?3,?4,?5,?6)",
            rusqlite::params![
                context.idempotency_id.as_str(),
                ordinal_index_i64,
                request.link_type,
                request.id,
                expected_version,
                classification,
            ],
        )
        .map_err(|_| storage_error(context))?;
    }
    let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(context))?;
    if tx
        .execute(
            "UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2",
            rusqlite::params![expected_revision + 1, expected_revision],
        )
        .map_err(|_| storage_error(context))?
        != 1
    {
        return Err(storage_error(context));
    }
    let record = RelationshipRecord::rehydrate(RelationshipPersistenceRecord {
        id: relationship_id,
        kind,
        endpoints: resolved,
        purpose,
        classification,
        version: AggregateVersion::initial(),
        created_at: occurred_at,
        updated_at: occurred_at,
    })
    .map_err(|_| storage_error(context))?;
    Ok(MutationOutcome::from_persistence(
        record,
        vec![audit.id().clone()],
        AuditEffectScope::Complete,
    ))
}

pub(super) fn decode_provenance(
    kind: &str,
    reference: Option<String>,
    context: &OperationContext,
) -> Result<Provenance, DomainError> {
    match kind {
        "user_entered" => Ok(Provenance::UserEntered),
        "authoritative_transition" => Ok(Provenance::AuthoritativeTransition(
            ProvenanceReference::parse(reference.ok_or_else(|| storage_error(context))?)
                .map_err(|_| storage_error(context))?,
        )),
        "synthetic_fixture" => Ok(Provenance::SyntheticFixture(
            ProvenanceReference::parse(reference.ok_or_else(|| storage_error(context))?)
                .map_err(|_| storage_error(context))?,
        )),
        _ => Err(storage_error(context)),
    }
}

fn decode_stakeholder_outcome(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<MutationOutcome<StakeholderRecord>, DomainError> {
    let row = tx
        .query_row(
            "SELECT result_id,result_name,result_stakeholder_kind,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at FROM relationship_stakeholder_command_results WHERE idempotency_id=?1",
            [context.idempotency_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    let record = StakeholderRecord::rehydrate(StakeholderPersistenceRecord {
        id: StakeholderId::parse(row.0).map_err(|_| storage_error(context))?,
        name: pmc_domain::relationships::StakeholderName::parse(row.1)
            .map_err(|_| storage_error(context))?,
        kind: StakeholderKind::from_persisted(&row.2).map_err(|_| storage_error(context))?,
        classification: DataClassification::from_persisted(&row.3)
            .map_err(|_| storage_error(context))?,
        provenance: decode_provenance(&row.4, row.5, context)?,
        version: AggregateVersion::new(u64::try_from(row.6).map_err(|_| storage_error(context))?)
            .map_err(|_| storage_error(context))?,
        created_at: UtcTimestamp::from_unix_millis(row.7),
        updated_at: UtcTimestamp::from_unix_millis(row.8),
    })
    .map_err(|_| storage_error(context))?;
    let audit_event_ids: Vec<AuditEventId> = {
        let mut statement = tx
            .prepare(
                "SELECT audit_event_id FROM relationship_replay_audits WHERE idempotency_id=?1 ORDER BY ordinal",
            )
            .map_err(|_| storage_error(context))?;
        let rows = statement
            .query_map([context.idempotency_id.as_str()], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|_| storage_error(context))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| storage_error(context))?;
        rows.into_iter()
            .map(|id| AuditEventId::parse(id).map_err(|_| storage_error(context)))
            .collect::<Result<_, _>>()?
    };
    Ok(MutationOutcome::from_persistence(
        record,
        audit_event_ids,
        AuditEffectScope::Complete,
    ))
}

/// `relationship_link_command_endpoints`/`relationship_link_result_endpoints`
/// use `kpi`, while `aggregate_registry`/`relationship_endpoints` use
/// `kpi_definition` for the same endpoint kind -- see the note on
/// `EndpointRequest`. Every other endpoint kind's spelling is shared.
pub(super) fn aggregate_type_to_link_type(aggregate_type: &str) -> &'static str {
    match aggregate_type {
        "portfolio" => "portfolio",
        "product" => "product",
        "initiative" => "initiative",
        "roadmap" => "roadmap",
        "kpi_definition" => "kpi",
        "project" => "project",
        "milestone" => "milestone",
        "stakeholder" => "stakeholder",
        _ => "portfolio",
    }
}

pub(super) fn build_endpoint_snapshot(
    aggregate_type: &str,
    id: &str,
    version: AggregateVersion,
    classification: DataClassification,
    context: &OperationContext,
) -> Result<EndpointSnapshot, DomainError> {
    Ok(match aggregate_type {
        "portfolio" => EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
            PortfolioId::parse(id).map_err(|_| storage_error(context))?,
            version,
            classification,
        )),
        "product" => EndpointSnapshot::Product(ProductSnapshot::new(
            ProductId::parse(id).map_err(|_| storage_error(context))?,
            version,
            classification,
        )),
        "initiative" => EndpointSnapshot::Initiative(InitiativeSnapshot::new(
            InitiativeId::parse(id).map_err(|_| storage_error(context))?,
            version,
            classification,
        )),
        "roadmap" => EndpointSnapshot::Roadmap(RoadmapSnapshot::new(
            RoadmapId::parse(id).map_err(|_| storage_error(context))?,
            version,
            classification,
        )),
        "kpi_definition" => EndpointSnapshot::Kpi(KpiSnapshot::new(
            KpiId::parse(id).map_err(|_| storage_error(context))?,
            version,
            classification,
        )),
        "project" => EndpointSnapshot::Project(ProjectSnapshot::new(
            ProjectId::parse(id).map_err(|_| storage_error(context))?,
            version,
            classification,
        )),
        "stakeholder" => {
            EndpointSnapshot::Stakeholder(pmc_domain::relationships::StakeholderSnapshot::new(
                StakeholderId::parse(id).map_err(|_| storage_error(context))?,
                version,
                classification,
            ))
        }
        _ => return Err(storage_error(context)),
    })
}

pub(super) fn endpoint_snapshot_version(snapshot: &EndpointSnapshot) -> AggregateVersion {
    match snapshot {
        EndpointSnapshot::Portfolio(v) => v.version(),
        EndpointSnapshot::Product(v) => v.version(),
        EndpointSnapshot::Initiative(v) => v.version(),
        EndpointSnapshot::Roadmap(v) => v.version(),
        EndpointSnapshot::Kpi(v) => v.version(),
        EndpointSnapshot::Project(v) => v.version(),
        EndpointSnapshot::Milestone(v) => v.version(),
        EndpointSnapshot::Stakeholder(v) => v.version(),
    }
}

pub(super) fn endpoint_snapshot_classification(snapshot: &EndpointSnapshot) -> DataClassification {
    match snapshot {
        EndpointSnapshot::Portfolio(v) => v.classification(),
        EndpointSnapshot::Product(v) => v.classification(),
        EndpointSnapshot::Initiative(v) => v.classification(),
        EndpointSnapshot::Roadmap(v) => v.classification(),
        EndpointSnapshot::Kpi(v) => v.classification(),
        EndpointSnapshot::Project(v) => v.classification(),
        EndpointSnapshot::Milestone(v) => v.classification(),
        EndpointSnapshot::Stakeholder(v) => v.classification(),
    }
}

fn endpoint_snapshot_audit_target(snapshot: &EndpointSnapshot) -> AuditTarget {
    match snapshot {
        EndpointSnapshot::Portfolio(v) => AuditTarget::Portfolio(v.id().clone()),
        EndpointSnapshot::Product(v) => AuditTarget::Product(v.id().clone()),
        EndpointSnapshot::Initiative(v) => AuditTarget::Initiative(v.id().clone()),
        EndpointSnapshot::Roadmap(v) => AuditTarget::Roadmap(v.id().clone()),
        EndpointSnapshot::Kpi(v) => AuditTarget::Kpi(v.id().clone()),
        EndpointSnapshot::Project(v) => AuditTarget::Project(v.id().clone()),
        EndpointSnapshot::Milestone(v) => AuditTarget::Milestone(v.id().clone()),
        EndpointSnapshot::Stakeholder(v) => AuditTarget::Stakeholder(v.id().clone()),
    }
}

fn relationship_code(kind: RelationshipKind) -> &'static str {
    match kind {
        RelationshipKind::PortfolioProduct => "relationship.portfolio_product.linked",
        RelationshipKind::PortfolioInitiative => "relationship.portfolio_initiative.linked",
        RelationshipKind::ProductRoadmap => "relationship.product_roadmap.linked",
        RelationshipKind::ProductKpi => "relationship.product_kpi.linked",
        RelationshipKind::InitiativeProject => "relationship.initiative_project.linked",
        RelationshipKind::ProjectProduct => "relationship.project_product.linked",
        RelationshipKind::StakeholderSubject => "relationship.stakeholder_subject.linked",
    }
}

fn decode_relationship_outcome(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<MutationOutcome<RelationshipRecord>, DomainError> {
    let (relationship_id, kind, purpose, result_version, result_classification, result_created_at, result_updated_at, effect_scope) : (String, String, Option<String>, i64, String, i64, i64, String) = tx
        .query_row(
            "SELECT relationship_id,relationship_kind,purpose,result_version,result_classification,result_created_at,result_updated_at,effect_scope FROM relationship_link_command_results WHERE idempotency_id=?1",
            [context.idempotency_id.as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    let endpoints: Vec<(String, String, i64)> = {
        let mut statement = tx
            .prepare("SELECT endpoint_type,endpoint_id,endpoint_version FROM relationship_link_result_endpoints WHERE idempotency_id=?1 ORDER BY ordinal")
            .map_err(|_| storage_error(context))?;
        let rows = statement
            .query_map([context.idempotency_id.as_str()], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|_| storage_error(context))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| storage_error(context))?;
        rows
    };
    let mut snapshots = Vec::with_capacity(endpoints.len());
    for (link_type, id, version) in &endpoints {
        let aggregate_type = link_type_to_aggregate_type(link_type);
        let (_, endpoint_classification): (i64, String) = tx
            .query_row(
                "SELECT version,classification FROM aggregate_registry WHERE id=?1 AND aggregate_type=?2",
                [id.as_str(), aggregate_type],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| storage_error(context))?;
        let classification = DataClassification::from_persisted(&endpoint_classification)
            .map_err(|_| storage_error(context))?;
        let version =
            AggregateVersion::new(u64::try_from(*version).map_err(|_| storage_error(context))?)
                .map_err(|_| storage_error(context))?;
        snapshots.push(build_endpoint_snapshot(
            aggregate_type,
            id,
            version,
            classification,
            context,
        )?);
    }
    let record = RelationshipRecord::rehydrate(RelationshipPersistenceRecord {
        id: RelationshipId::parse(relationship_id).map_err(|_| storage_error(context))?,
        kind: RelationshipKind::from_persisted(&kind).map_err(|_| storage_error(context))?,
        endpoints: snapshots,
        purpose: purpose
            .map(|value| {
                pmc_domain::relationships::StakeholderRelationshipPurpose::from_persisted(&value)
                    .map_err(|_| storage_error(context))
            })
            .transpose()?,
        classification: DataClassification::from_persisted(&result_classification)
            .map_err(|_| storage_error(context))?,
        version: AggregateVersion::new(
            u64::try_from(result_version).map_err(|_| storage_error(context))?,
        )
        .map_err(|_| storage_error(context))?,
        created_at: UtcTimestamp::from_unix_millis(result_created_at),
        updated_at: UtcTimestamp::from_unix_millis(result_updated_at),
    })
    .map_err(|_| storage_error(context))?;
    let audit_event_ids: Vec<AuditEventId> = if effect_scope == "none" {
        Vec::new()
    } else {
        let mut statement = tx
            .prepare("SELECT audit_event_id FROM relationship_replay_audits WHERE idempotency_id=?1 ORDER BY ordinal")
            .map_err(|_| storage_error(context))?;
        let rows = statement
            .query_map([context.idempotency_id.as_str()], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|_| storage_error(context))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| storage_error(context))?;
        rows.into_iter()
            .map(|id| AuditEventId::parse(id).map_err(|_| storage_error(context)))
            .collect::<Result<_, _>>()?
    };
    let scope = match effect_scope.as_str() {
        "none" => AuditEffectScope::None,
        "partial" => AuditEffectScope::Partial,
        _ => AuditEffectScope::Complete,
    };
    Ok(MutationOutcome::from_persistence(
        record,
        audit_event_ids,
        scope,
    ))
}

pub(super) fn link_type_to_aggregate_type(link_type: &str) -> &'static str {
    match link_type {
        "kpi" => "kpi_definition",
        "portfolio" => "portfolio",
        "product" => "product",
        "initiative" => "initiative",
        "roadmap" => "roadmap",
        "project" => "project",
        "milestone" => "milestone",
        "stakeholder" => "stakeholder",
        _ => "portfolio",
    }
}

pub(super) fn build_audit(
    id: AuditEventId,
    at: UtcTimestamp,
    target: AuditTarget,
    code: &str,
    context: &OperationContext,
) -> Result<AuditEvent, DomainError> {
    Ok(AuditEvent::new(
        id,
        at,
        AuditActor::HeadOfProducts,
        AuditAction::new(
            AuditModule::Portfolio,
            AuditEventCode::parse(code).map_err(|_| storage_error(context))?,
            target,
        ),
        context.correlation_id.clone(),
        AuditDisposition::new(
            AuditPolicyOutcome::NotRequired,
            pmc_domain::audit::AuditApprovalOutcome::NotRequired,
            AuditExecutionOutcome::Succeeded,
            AuditEffectScope::Complete,
            vec![AuditEffectCode::parse(RELATIONSHIP_EFFECT_CODE)
                .map_err(|_| storage_error(context))?],
        )
        .map_err(|_| storage_error(context))?,
    ))
}

pub(super) fn persist_relationship_audit(
    tx: &Transaction<'_>,
    audit: &AuditEvent,
    target_type: &str,
    target_id: &str,
    context: &OperationContext,
) -> Result<(), DomainError> {
    tx.execute(
        "INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES(?1,?2,'head_of_products','portfolio',?3,?4,?5,?6,'not_required','not_required','succeeded','complete')",
        rusqlite::params![
            audit.id().as_str(),
            audit.occurred_at().unix_millis(),
            audit.code().as_str(),
            target_type,
            target_id,
            context.correlation_id.as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO audit_effects(audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES(?1,0,?2,'complete',?3,?4)",
        rusqlite::params![audit.id().as_str(), RELATIONSHIP_EFFECT_CODE, target_type, target_id],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

pub(super) fn next_relationship_operation_ordinal(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,1) FROM relationship_replay_operations",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

pub(super) fn storage_error(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::PlatformInternal,
        MessageKey::parse("relationship.persistence_failed").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        true,
    )
}

pub(super) fn idempotency_conflict(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainIdempotencyConflict,
        MessageKey::parse("relationship.idempotency_conflict").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

pub(super) fn domain_conflict(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainConflict,
        MessageKey::parse("relationship.conflict").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

pub(super) fn domain_not_found(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainNotFound,
        MessageKey::parse("relationship.not_found").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

fn milestone_subject_not_supported(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::ValidationInvalidField,
        MessageKey::parse("relationship.milestone_subject_not_supported")
            .unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

pub(super) fn policy_denied(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::SecurityPolicyDenied,
        MessageKey::parse("relationship.classification.unclassified_or_lowering_denied")
            .unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}
