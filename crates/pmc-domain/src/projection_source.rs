//! Read-only projection-source snapshot: a narrow,
//! versioned view over the six Ledger-authoritative record types that
//! Product Vault projections are permitted to render: these DTOs are the
//! field allowlist.
//!
//! This is deliberately NOT a serialization of any domain record,
//! persistence snapshot, or SQL row: every field here is a field the
//! projection allowlist explicitly permits (stable identity, canonical
//! type, lifecycle, approved relationship identifiers, due/observed time,
//! classification, source revision). No user-authored free text (title,
//! details, statement, rationale, support, ...) belongs on these types --
//! adding a field here is itself the allowlist review the plan requires.
//!
//! `LedgerSnapshotForProjectionPort` is the one operation a projection
//! generator may use to read Ledger state. It returns every collection from
//! one consistent point in time so a generator never composes six
//! independently-read, potentially revision-inconsistent lists.

use crate::classification::DataClassification;
use crate::identity::{
    ActionId, ActionRequestId, AggregateVersion, DecisionId, KpiId, ProductId, ProjectId, RiskId,
};
use crate::time::UtcTimestamp;
use crate::work_management::{ActionState, DecisionState, RiskState};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProductProjectionSource {
    pub id: ProductId,
    pub classification: DataClassification,
    pub source_revision: AggregateVersion,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectProjectionSource {
    pub id: ProjectId,
    pub classification: DataClassification,
    pub source_revision: AggregateVersion,
    pub start_at: UtcTimestamp,
    pub end_at: UtcTimestamp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionProjectionSource {
    pub id: ActionId,
    pub classification: DataClassification,
    pub source_revision: AggregateVersion,
    pub state: ActionState,
    pub due_at: UtcTimestamp,
    pub source_request_id: ActionRequestId,
    pub source_decision_id: Option<DecisionId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionProjectionSource {
    pub id: DecisionId,
    pub classification: DataClassification,
    pub source_revision: AggregateVersion,
    pub state: DecisionState,
    pub decided_at: UtcTimestamp,
    pub supersedes_decision_id: Option<DecisionId>,
    pub superseded_by_decision_id: Option<DecisionId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskProjectionSource {
    pub id: RiskId,
    pub classification: DataClassification,
    pub source_revision: AggregateVersion,
    pub state: RiskState,
    pub next_review_at: Option<UtcTimestamp>,
}

/// KPI Definition only -- never the measured value on a `KpiObservationRecord`,
/// which the projection allowlist excludes to avoid leaking a potentially sensitive
/// number into a generated file. Observation freshness is a later, separately
/// reviewed addition (see the projection field-allowlist review requirement).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KpiProjectionSource {
    pub id: KpiId,
    pub classification: DataClassification,
    pub source_revision: AggregateVersion,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LedgerProjectionSnapshot {
    pub schema_version: u32,
    pub ledger_revision: u64,
    pub ledger_as_of_utc: UtcTimestamp,
    pub products: Vec<ProductProjectionSource>,
    pub projects: Vec<ProjectProjectionSource>,
    pub actions: Vec<ActionProjectionSource>,
    pub decisions: Vec<DecisionProjectionSource>,
    pub risks: Vec<RiskProjectionSource>,
    pub kpis: Vec<KpiProjectionSource>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionSnapshotReadError {
    Unavailable,
    ReadFailed,
}

/// One consistent read of every projectable record type. An implementation
/// must read all six collections from the same point in Ledger history --
/// never compose them from separate, independently-timed reads -- and order
/// each collection deterministically by its own stable ID so repeated reads
/// of unchanged state are byte-for-byte comparable.
///
/// `observed_at` becomes `ledger_as_of_utc` on the returned envelope
/// unchanged. The caller supplies it rather than the port reading a wall
/// clock itself, matching this crate's existing rule that `pmc-ledger` never
/// originates a timestamp -- every other command/query timestamp already
/// arrives from its caller.
pub trait LedgerSnapshotForProjectionPort {
    fn read_projection_snapshot(
        &self,
        observed_at: UtcTimestamp,
    ) -> Result<LedgerProjectionSnapshot, ProjectionSnapshotReadError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_snapshot() -> LedgerProjectionSnapshot {
        LedgerProjectionSnapshot {
            schema_version: 40,
            ledger_revision: 1,
            ledger_as_of_utc: UtcTimestamp::from_unix_millis(0),
            products: Vec::new(),
            projects: Vec::new(),
            actions: Vec::new(),
            decisions: Vec::new(),
            risks: Vec::new(),
            kpis: Vec::new(),
        }
    }

    /// A regression against ever widening these types back into whole-record
    /// serialization: this test only compiles because every field on every
    /// `*ProjectionSource` type is a type this test module can name without
    /// importing any free-text value type (`ActionTitle`, `ActionDetails`,
    /// `DecisionText`, `RiskTitle`, `RiskDetails`, `ShortText`, `LongText`).
    #[test]
    fn an_empty_snapshot_round_trips_through_equality() {
        let first = empty_snapshot();
        let second = empty_snapshot();
        assert_eq!(first, second);
    }

    #[test]
    fn two_snapshots_differing_only_by_ledger_revision_are_unequal() {
        let mut other = empty_snapshot();
        other.ledger_revision = 2;
        assert_ne!(empty_snapshot(), other);
    }
}
