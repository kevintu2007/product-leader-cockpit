//! SQLite implementation of `LedgerSnapshotForProjectionPort`: the one
//! Ledger-side read a managed projection generator may use. Reads
//! every projectable record type inside one transaction so the returned
//! envelope is a single consistent point in Ledger history, never six
//! independently-timed reads.
//!
//! Every query here selects only the columns `pmc_domain::projection_source`
//! declares as allowlisted -- never `SELECT *`, never a free-text column
//! (`title`, `details`, `statement`, `rationale`, `name`, `definition`, ...).
//! Widening a query here to select a new column is itself the allowlist
//! review managed projections require; it must not happen incidentally.

use pmc_domain::{
    classification::DataClassification,
    identity::{
        ActionId, ActionRequestId, AggregateVersion, DecisionId, KpiId, ProductId, ProjectId,
        RiskId,
    },
    projection_source::{
        ActionProjectionSource, DecisionProjectionSource, KpiProjectionSource,
        LedgerProjectionSnapshot, LedgerSnapshotForProjectionPort, ProductProjectionSource,
        ProjectProjectionSource, ProjectionSnapshotReadError, RiskProjectionSource,
    },
    time::UtcTimestamp,
    work_management::{ActionState, DecisionState, RiskState},
};
use rusqlite::Transaction;

use super::{SqliteProductLedger, CURRENT_SCHEMA_VERSION};

/// Decodes the shared `(classification, version)` pair every projectable
/// table's `aggregate_registry` join carries, in the fixed column order
/// every query below places them last.
fn decode_classification_and_revision(
    classification: &str,
    version: i64,
) -> Result<(DataClassification, AggregateVersion), ProjectionSnapshotReadError> {
    let classification = DataClassification::from_persisted(classification)
        .map_err(|_| ProjectionSnapshotReadError::ReadFailed)?;
    let version = u64::try_from(version).map_err(|_| ProjectionSnapshotReadError::ReadFailed)?;
    let version =
        AggregateVersion::new(version).map_err(|_| ProjectionSnapshotReadError::ReadFailed)?;
    Ok((classification, version))
}

fn read_products(
    tx: &Transaction<'_>,
) -> Result<Vec<ProductProjectionSource>, ProjectionSnapshotReadError> {
    tx.prepare(
        "SELECT products.id,registry.classification,registry.version \
         FROM products JOIN aggregate_registry registry \
         ON registry.id=products.id AND registry.aggregate_type='product' \
         ORDER BY products.id",
    )
    .map_err(|_| ProjectionSnapshotReadError::ReadFailed)?
    .query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })
    .map_err(|_| ProjectionSnapshotReadError::ReadFailed)?
    .map(|row| {
        let (id, classification, version) =
            row.map_err(|_| ProjectionSnapshotReadError::ReadFailed)?;
        let id = ProductId::parse(id).map_err(|_| ProjectionSnapshotReadError::ReadFailed)?;
        let (classification, source_revision) =
            decode_classification_and_revision(&classification, version)?;
        Ok(ProductProjectionSource {
            id,
            classification,
            source_revision,
        })
    })
    .collect()
}

fn read_projects(
    tx: &Transaction<'_>,
) -> Result<Vec<ProjectProjectionSource>, ProjectionSnapshotReadError> {
    tx.prepare(
        "SELECT projects.id,projects.start_at,projects.end_at,registry.classification,registry.version \
         FROM projects JOIN aggregate_registry registry \
         ON registry.id=projects.id AND registry.aggregate_type='project' \
         ORDER BY projects.id",
    )
    .map_err(|_| ProjectionSnapshotReadError::ReadFailed)?
    .query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })
    .map_err(|_| ProjectionSnapshotReadError::ReadFailed)?
    .map(|row| {
        let (id, start_at, end_at, classification, version) =
            row.map_err(|_| ProjectionSnapshotReadError::ReadFailed)?;
        let id = ProjectId::parse(id).map_err(|_| ProjectionSnapshotReadError::ReadFailed)?;
        let (classification, source_revision) =
            decode_classification_and_revision(&classification, version)?;
        Ok(ProjectProjectionSource {
            id,
            classification,
            source_revision,
            start_at: UtcTimestamp::from_unix_millis(start_at),
            end_at: UtcTimestamp::from_unix_millis(end_at),
        })
    })
    .collect()
}

fn read_actions(
    tx: &Transaction<'_>,
) -> Result<Vec<ActionProjectionSource>, ProjectionSnapshotReadError> {
    tx.prepare(
        "SELECT actions.id,actions.source_request_id,actions.due_at,actions.state,\
         actions.source_decision_id,registry.classification,registry.version \
         FROM actions JOIN aggregate_registry registry \
         ON registry.id=actions.id AND registry.aggregate_type='action' \
         ORDER BY actions.id",
    )
    .map_err(|_| ProjectionSnapshotReadError::ReadFailed)?
    .query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, i64>(6)?,
        ))
    })
    .map_err(|_| ProjectionSnapshotReadError::ReadFailed)?
    .map(|row| {
        let (id, source_request_id, due_at, state, source_decision_id, classification, version) =
            row.map_err(|_| ProjectionSnapshotReadError::ReadFailed)?;
        let id = ActionId::parse(id).map_err(|_| ProjectionSnapshotReadError::ReadFailed)?;
        let source_request_id = ActionRequestId::parse(source_request_id)
            .map_err(|_| ProjectionSnapshotReadError::ReadFailed)?;
        let state = ActionState::from_persisted(&state)
            .map_err(|_| ProjectionSnapshotReadError::ReadFailed)?;
        let source_decision_id = source_decision_id
            .map(DecisionId::parse)
            .transpose()
            .map_err(|_| ProjectionSnapshotReadError::ReadFailed)?;
        let (classification, source_revision) =
            decode_classification_and_revision(&classification, version)?;
        Ok(ActionProjectionSource {
            id,
            classification,
            source_revision,
            state,
            due_at: UtcTimestamp::from_unix_millis(due_at),
            source_request_id,
            source_decision_id,
        })
    })
    .collect()
}

fn read_decisions(
    tx: &Transaction<'_>,
) -> Result<Vec<DecisionProjectionSource>, ProjectionSnapshotReadError> {
    tx.prepare(
        "SELECT decisions.id,decisions.decided_at,decisions.state,\
         decisions.supersedes_decision_id,decisions.superseded_by_decision_id,\
         registry.classification,registry.version \
         FROM decisions JOIN aggregate_registry registry \
         ON registry.id=decisions.id AND registry.aggregate_type='decision' \
         ORDER BY decisions.id",
    )
    .map_err(|_| ProjectionSnapshotReadError::ReadFailed)?
    .query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, i64>(6)?,
        ))
    })
    .map_err(|_| ProjectionSnapshotReadError::ReadFailed)?
    .map(|row| {
        let (
            id,
            decided_at,
            state,
            supersedes_decision_id,
            superseded_by_decision_id,
            classification,
            version,
        ) = row.map_err(|_| ProjectionSnapshotReadError::ReadFailed)?;
        let id = DecisionId::parse(id).map_err(|_| ProjectionSnapshotReadError::ReadFailed)?;
        let state = DecisionState::from_persisted(&state)
            .map_err(|_| ProjectionSnapshotReadError::ReadFailed)?;
        let supersedes_decision_id = supersedes_decision_id
            .map(DecisionId::parse)
            .transpose()
            .map_err(|_| ProjectionSnapshotReadError::ReadFailed)?;
        let superseded_by_decision_id = superseded_by_decision_id
            .map(DecisionId::parse)
            .transpose()
            .map_err(|_| ProjectionSnapshotReadError::ReadFailed)?;
        let (classification, source_revision) =
            decode_classification_and_revision(&classification, version)?;
        Ok(DecisionProjectionSource {
            id,
            classification,
            source_revision,
            state,
            decided_at: UtcTimestamp::from_unix_millis(decided_at),
            supersedes_decision_id,
            superseded_by_decision_id,
        })
    })
    .collect()
}

fn read_risks(
    tx: &Transaction<'_>,
) -> Result<Vec<RiskProjectionSource>, ProjectionSnapshotReadError> {
    tx.prepare(
        "SELECT risks.id,risks.state,risks.next_review_at,registry.classification,registry.version \
         FROM risks JOIN aggregate_registry registry \
         ON registry.id=risks.id AND registry.aggregate_type='risk' \
         ORDER BY risks.id",
    )
    .map_err(|_| ProjectionSnapshotReadError::ReadFailed)?
    .query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })
    .map_err(|_| ProjectionSnapshotReadError::ReadFailed)?
    .map(|row| {
        let (id, state, next_review_at, classification, version) =
            row.map_err(|_| ProjectionSnapshotReadError::ReadFailed)?;
        let id = RiskId::parse(id).map_err(|_| ProjectionSnapshotReadError::ReadFailed)?;
        let state =
            RiskState::from_persisted(&state).map_err(|_| ProjectionSnapshotReadError::ReadFailed)?;
        let (classification, source_revision) =
            decode_classification_and_revision(&classification, version)?;
        Ok(RiskProjectionSource {
            id,
            classification,
            source_revision,
            state,
            next_review_at: next_review_at.map(UtcTimestamp::from_unix_millis),
        })
    })
    .collect()
}

fn read_kpis(
    tx: &Transaction<'_>,
) -> Result<Vec<KpiProjectionSource>, ProjectionSnapshotReadError> {
    tx.prepare(
        "SELECT kpi_definitions.id,registry.classification,registry.version \
         FROM kpi_definitions JOIN aggregate_registry registry \
         ON registry.id=kpi_definitions.id AND registry.aggregate_type='kpi_definition' \
         ORDER BY kpi_definitions.id",
    )
    .map_err(|_| ProjectionSnapshotReadError::ReadFailed)?
    .query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })
    .map_err(|_| ProjectionSnapshotReadError::ReadFailed)?
    .map(|row| {
        let (id, classification, version) =
            row.map_err(|_| ProjectionSnapshotReadError::ReadFailed)?;
        let id = KpiId::parse(id).map_err(|_| ProjectionSnapshotReadError::ReadFailed)?;
        let (classification, source_revision) =
            decode_classification_and_revision(&classification, version)?;
        Ok(KpiProjectionSource {
            id,
            classification,
            source_revision,
        })
    })
    .collect()
}

impl LedgerSnapshotForProjectionPort for SqliteProductLedger {
    fn read_projection_snapshot(
        &self,
        observed_at: UtcTimestamp,
    ) -> Result<LedgerProjectionSnapshot, ProjectionSnapshotReadError> {
        if self.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(ProjectionSnapshotReadError::Unavailable);
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|_| ProjectionSnapshotReadError::Unavailable)?;
        let ledger_revision: i64 = transaction
            .query_row(
                "SELECT ledger_revision FROM ledger_metadata WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .map_err(|_| ProjectionSnapshotReadError::ReadFailed)?;
        let ledger_revision =
            u64::try_from(ledger_revision).map_err(|_| ProjectionSnapshotReadError::ReadFailed)?;
        let snapshot = LedgerProjectionSnapshot {
            schema_version: self.schema_version,
            ledger_revision,
            ledger_as_of_utc: observed_at,
            products: read_products(&transaction)?,
            projects: read_projects(&transaction)?,
            actions: read_actions(&transaction)?,
            decisions: read_decisions(&transaction)?,
            risks: read_risks(&transaction)?,
            kpis: read_kpis(&transaction)?,
        };
        transaction
            .commit()
            .map_err(|_| ProjectionSnapshotReadError::Unavailable)?;
        Ok(snapshot)
    }
}
