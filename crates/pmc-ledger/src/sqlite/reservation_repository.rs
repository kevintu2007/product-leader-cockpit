//! Durable record-id reservations (schema v47, Evidence since v48; DG3 record-entry amendment
//! §3.6 and §4; product owner 2026-09-21 and 2026-09-22).
//!
//! A record-entry sheet sends one `clientRequestId` and reuses it verbatim on
//! a retry, so a retry can never create a second record. The host mints the
//! new record's id — but every create writer stores and compares its whole
//! command, id included, so an id minted afresh on the retry would be a
//! different command. The id is therefore reserved here first, durably and
//! in its own committed transaction, keyed by that `clientRequestId`
//! (the idempotency id): the first attempt mints and stores it; a retry —
//! after a crash before the create committed, or after it — reads the same
//! id back and so builds the same command.
//!
//! A reservation is not a claim: `ledger_idempotency_claims` is filled by
//! each command's own replay row, so the reservation checks the claims table
//! but never writes it. The reservation advances `ledger_revision` (product
//! owner, 2026-09-22): it is a durable change to what a retry will do.

use pmc_domain::identity::IdempotencyId;
use pmc_domain::time::UtcTimestamp;
use pmc_domain::DomainValueError;
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior};

use super::SqliteProductLedger;

/// The kinds of record a reservation can name: the `aggregate_registry`
/// types a record-entry sheet may create.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReservedEntityKind {
    Portfolio,
    Product,
    Initiative,
    Project,
    Roadmap,
    Milestone,
    KpiDefinition,
    KpiObservation,
    Stakeholder,
    ActionRequest,
    DecisionRequest,
    Risk,
    Issue,
    Relationship,
    /// Evidence from a file (schema v48).
    EvidenceReference,
}

impl ReservedEntityKind {
    #[must_use]
    pub const fn as_persisted(self) -> &'static str {
        match self {
            Self::Portfolio => "portfolio",
            Self::Product => "product",
            Self::Initiative => "initiative",
            Self::Project => "project",
            Self::Roadmap => "roadmap",
            Self::Milestone => "milestone",
            Self::KpiDefinition => "kpi_definition",
            Self::KpiObservation => "kpi_observation",
            Self::Stakeholder => "stakeholder",
            Self::ActionRequest => "action_request",
            Self::DecisionRequest => "decision_request",
            Self::Risk => "risk",
            Self::Issue => "issue",
            Self::Relationship => "relationship",
            Self::EvidenceReference => "evidence_reference",
        }
    }

    /// The idempotency namespace the record's own create command claims, so
    /// a reservation and the command that follows it agree.
    #[must_use]
    pub const fn namespace(self) -> &'static str {
        match self {
            Self::Portfolio
            | Self::Product
            | Self::Roadmap
            | Self::KpiDefinition
            | Self::KpiObservation => "portfolio",
            Self::Initiative | Self::Project | Self::Milestone => "delivery",
            Self::Stakeholder => "people",
            Self::ActionRequest => "action",
            Self::DecisionRequest => "decision",
            Self::Risk => "risk",
            Self::Issue => "issue",
            Self::Relationship => "relationship",
            Self::EvidenceReference => "evidence",
        }
    }
}

/// What a reservation names: the kind and the create operation. Together
/// with the idempotency id these are the reservation's identity; the same
/// idempotency id for anything else is a conflict.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReservationRequest {
    pub kind: ReservedEntityKind,
    /// The create operation's persisted name, e.g. `create_risk`.
    pub operation: &'static str,
}

/// A typed record id that this Ledger holds durably for one idempotency id.
/// Only [`SqliteProductLedger::reserve_or_get_record_id`] makes one; a create
/// writer that takes it can only be reached through a reservation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReservedId<T> {
    idempotency_id: IdempotencyId,
    request: ReservationRequest,
    id: T,
    replayed: bool,
}

impl<T> ReservedId<T> {
    pub fn id(&self) -> &T {
        &self.id
    }

    pub fn idempotency_id(&self) -> &IdempotencyId {
        &self.idempotency_id
    }

    pub const fn request(&self) -> ReservationRequest {
        self.request
    }

    /// True when the id was already reserved by an earlier attempt.
    pub const fn replayed(&self) -> bool {
        self.replayed
    }
}

/// A typed record id the reservation can store and read back.
pub trait ReservableId: Sized {
    fn as_str(&self) -> &str;
    fn parse_reserved(value: String) -> Result<Self, DomainValueError>;
}

macro_rules! reservable {
    ($($id:ty),+ $(,)?) => {
        $(impl ReservableId for $id {
            fn as_str(&self) -> &str {
                self.as_str()
            }
            fn parse_reserved(value: String) -> Result<Self, DomainValueError> {
                <$id>::parse(value)
            }
        })+
    };
}

reservable!(
    pmc_domain::identity::PortfolioId,
    pmc_domain::identity::ProductId,
    pmc_domain::identity::InitiativeId,
    pmc_domain::identity::ProjectId,
    pmc_domain::identity::RoadmapId,
    pmc_domain::identity::MilestoneId,
    pmc_domain::identity::KpiId,
    pmc_domain::identity::KpiObservationId,
    pmc_domain::identity::StakeholderId,
    pmc_domain::identity::ActionRequestId,
    pmc_domain::identity::DecisionRequestId,
    pmc_domain::identity::RiskId,
    pmc_domain::identity::IssueId,
    pmc_domain::identity::RelationshipId,
    pmc_domain::identity::EvidenceReferenceId,
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReservationError {
    /// The idempotency id is reserved for another kind or operation, or was
    /// already claimed by a command that reserved nothing.
    Conflict,
    /// A stored id no longer parses as the requested kind: corruption.
    Corrupt,
    /// The minter returned an invalid id, or only ids already in use.
    Mint,
    /// This Ledger is not at the current schema.
    IncompatibleLedger,
    /// SQLite refused or failed.
    Storage,
}

const MINT_ATTEMPTS: u32 = 8;

impl SqliteProductLedger {
    /// Reserve a record id for `idempotency_id`, or read back the one an
    /// earlier attempt reserved. Its own committed transaction (see the
    /// module doc); the create transaction follows separately.
    pub fn reserve_or_get_record_id<T: ReservableId>(
        &mut self,
        idempotency_id: &IdempotencyId,
        request: ReservationRequest,
        reserved_at: UtcTimestamp,
        mut mint: impl FnMut() -> Result<T, DomainValueError>,
    ) -> Result<ReservedId<T>, ReservationError> {
        if self.schema_version != super::CURRENT_SCHEMA_VERSION {
            return Err(ReservationError::IncompatibleLedger);
        }
        let expected_revision = self.revision().map_err(|_| ReservationError::Storage)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| ReservationError::Storage)?;
        if let Some(existing) = read_reservation(&transaction, idempotency_id)? {
            if existing.0 != request.kind.namespace()
                || existing.1 != request.operation
                || existing.2 != request.kind.as_persisted()
            {
                return Err(ReservationError::Conflict);
            }
            let id = T::parse_reserved(existing.3).map_err(|_| ReservationError::Corrupt)?;
            return Ok(ReservedId {
                idempotency_id: idempotency_id.clone(),
                request,
                id,
                replayed: true,
            });
        }
        let claimed: i64 = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM ledger_idempotency_claims WHERE idempotency_id=?1)",
                [idempotency_id.as_str()],
                |row| row.get(0),
            )
            .map_err(|_| ReservationError::Storage)?;
        if claimed != 0 {
            return Err(ReservationError::Conflict);
        }
        let mut minted = None;
        for _ in 0..MINT_ATTEMPTS {
            let candidate = mint().map_err(|_| ReservationError::Mint)?;
            let in_use: i64 = transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM record_id_reservations WHERE generated_id=?1) OR EXISTS(SELECT 1 FROM aggregate_registry WHERE id=?1)",
                    [candidate.as_str()],
                    |row| row.get(0),
                )
                .map_err(|_| ReservationError::Storage)?;
            if in_use == 0 {
                minted = Some(candidate);
                break;
            }
        }
        let id = minted.ok_or(ReservationError::Mint)?;
        transaction
            .execute(
                "INSERT INTO record_id_reservations(idempotency_id,namespace,operation,entity_kind,generated_id,reserved_at) VALUES(?1,?2,?3,?4,?5,?6)",
                rusqlite::params![
                    idempotency_id.as_str(),
                    request.kind.namespace(),
                    request.operation,
                    request.kind.as_persisted(),
                    id.as_str(),
                    reserved_at.unix_millis(),
                ],
            )
            .map_err(|_| ReservationError::Storage)?;
        let revision = i64::try_from(expected_revision).map_err(|_| ReservationError::Storage)?;
        if transaction
            .execute(
                "UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2",
                rusqlite::params![revision + 1, revision],
            )
            .map_err(|_| ReservationError::Storage)?
            != 1
        {
            return Err(ReservationError::Storage);
        }
        transaction
            .commit()
            .map_err(|_| ReservationError::Storage)?;
        Ok(ReservedId {
            idempotency_id: idempotency_id.clone(),
            request,
            id,
            replayed: false,
        })
    }
}

fn read_reservation(
    transaction: &Transaction<'_>,
    idempotency_id: &IdempotencyId,
) -> Result<Option<(String, String, String, String)>, ReservationError> {
    transaction
        .query_row(
            "SELECT namespace,operation,entity_kind,generated_id FROM record_id_reservations WHERE idempotency_id=?1",
            [idempotency_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|_| ReservationError::Storage)
}

/// Inside a create transaction: the reservation for this idempotency id
/// exists and names exactly this kind, operation and id. A create writer
/// that takes a [`ReservedId`] calls this before it writes, so a forged or
/// mismatched proof — and a command whose id is not the reserved one — is
/// refused. `Ok(false)` means no such reservation.
pub(super) fn reservation_matches<T: ReservableId>(
    connection: &rusqlite::Connection,
    reserved: &ReservedId<T>,
) -> Result<bool, rusqlite::Error> {
    let found: i64 = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM record_id_reservations WHERE idempotency_id=?1 AND namespace=?2 AND operation=?3 AND entity_kind=?4 AND generated_id=?5)",
        rusqlite::params![
            reserved.idempotency_id.as_str(),
            reserved.request.kind.namespace(),
            reserved.request.operation,
            reserved.request.kind.as_persisted(),
            reserved.id.as_str(),
        ],
        |row| row.get(0),
    )?;
    Ok(found != 0)
}
