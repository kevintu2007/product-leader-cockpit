//! The shared H1 record-entry flow (slice 6A; DG3 record-entry amendment
//! §3.6, §4; product owner 2026-09-21/22): what every create sheet's command
//! does before its own writer runs.
//!
//! The webview supplies the fields the person entered and one
//! `clientRequestId` per opened sheet. The host mints the new record's id —
//! through the Ledger's durable reservation, so a retry of that request
//! builds the same command and names the same record — plus the correlation,
//! audit id and instant. Provenance is `UserEntered` wherever the domain
//! records one; the webview cannot choose it.
//!
//! One function per create lands with its record family (6B–6E). Slice 6A
//! ships the reservation step, the first create that uses it (Risk, whose
//! sheet arrives in 6E) and the error classes every sheet maps the same way.

#![allow(clippy::result_large_err)]

use pmc_domain::classification::DataClassification;
use pmc_domain::error::{DomainError, ErrorCode};
use pmc_domain::identity::{IdempotencyId, RiskId};
use pmc_domain::risks::{
    RiskDetails, RiskMutationOutcome, RiskOperationContext, RiskRecord, RiskTitle,
};
use pmc_domain::time::UtcTimestamp;
use pmc_domain::DomainValueError;
use pmc_ledger::sqlite::{
    LedgerTransactionError, ReservableId, ReservationError, ReservationRequest, ReservedEntityKind,
    ReservedId, SqliteProductLedger,
};

use crate::desktop_runtime::OpaqueIdSource;

/// Why a record-entry command did not commit. Each maps to one stable safe
/// error class, the same for every sheet.
#[derive(Debug)]
pub enum RecordEntryError {
    /// The `clientRequestId` is reserved for another kind or operation, or
    /// was spent by a command that reserved nothing (`DOMAIN_IDEMPOTENCY_CONFLICT`).
    RequestReused,
    /// A stored reservation no longer parses: corruption, not retryable
    /// (`PLATFORM_INTERNAL`).
    ReservationCorrupt,
    /// The host's id source produced an invalid identifier: a host bug.
    Id(DomainValueError),
    /// The domain refused the command (its own error class and key).
    Domain(DomainError),
    /// The Ledger refused or failed the write.
    Ledger(LedgerTransactionError<DomainError>),
    /// This Ledger is not open at the current schema.
    IncompatibleLedger,
    /// SQLite could not reserve; retryable.
    Storage,
}

impl RecordEntryError {
    /// The safe error class the host reports.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::RequestReused => ErrorCode::DomainIdempotencyConflict,
            Self::ReservationCorrupt | Self::Id(_) | Self::IncompatibleLedger | Self::Storage => {
                ErrorCode::PlatformInternal
            }
            Self::Domain(error) => error.code(),
            Self::Ledger(LedgerTransactionError::Operation(error)) => error.code(),
            Self::Ledger(_) => ErrorCode::PlatformInternal,
        }
    }

    /// Whether the same request can be sent again unchanged.
    #[must_use]
    pub fn retryable(&self) -> bool {
        match self {
            Self::Storage => true,
            Self::Ledger(LedgerTransactionError::Busy | LedgerTransactionError::CommitFailed) => {
                true
            }
            Self::Domain(error) | Self::Ledger(LedgerTransactionError::Operation(error)) => {
                error.retryable()
            }
            _ => false,
        }
    }
}

impl From<ReservationError> for RecordEntryError {
    fn from(error: ReservationError) -> Self {
        match error {
            ReservationError::Conflict => Self::RequestReused,
            ReservationError::Corrupt => Self::ReservationCorrupt,
            ReservationError::Mint => Self::Id(DomainValueError::new(
                pmc_domain::ValueErrorKind::InvalidCharacter,
            )),
            ReservationError::IncompatibleLedger => Self::IncompatibleLedger,
            ReservationError::Storage => Self::Storage,
        }
    }
}

impl From<LedgerTransactionError<DomainError>> for RecordEntryError {
    fn from(error: LedgerTransactionError<DomainError>) -> Self {
        Self::Ledger(error)
    }
}

/// Reserve the id a create will use for `idempotency_id`, or read back the
/// one an earlier attempt reserved. `mint` is one of the host's typed
/// minters; the reservation retries it if the id is already in use.
pub fn reserve_record_id<T: ReservableId>(
    ledger: &mut SqliteProductLedger,
    idempotency_id: &IdempotencyId,
    request: ReservationRequest,
    now: UtcTimestamp,
    mint: impl FnMut() -> Result<T, DomainValueError>,
) -> Result<ReservedId<T>, RecordEntryError> {
    Ok(ledger.reserve_or_get_record_id(idempotency_id, request, now, mint)?)
}

/// The reservation every Risk create names.
pub const CREATE_RISK: ReservationRequest = ReservationRequest {
    kind: ReservedEntityKind::Risk,
    operation: "create_risk",
};

/// What a Risk sheet sends (6E); nothing here is minted by the webview.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskEntry {
    pub title: RiskTitle,
    pub details: RiskDetails,
    /// Chosen by the person; Risk refuses `Unclassified`.
    pub classification: DataClassification,
}

/// H1 create Risk: reserve (or recover) the id for this request, then the
/// reservation-checked create. A retry of the same request returns the Risk
/// the first attempt created; nothing is created twice.
pub fn enter_risk(
    ledger: &mut SqliteProductLedger,
    entry: RiskEntry,
    context: RiskOperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<RiskMutationOutcome<RiskRecord>, RecordEntryError> {
    let reserved: ReservedId<RiskId> =
        reserve_record_id(ledger, &context.idempotency_id, CREATE_RISK, now, || {
            ids.next_risk_id()
        })?;
    let audit_event_id = pmc_domain::audit::AuditEventIdSource::next_audit_event_id(ids)
        .map_err(RecordEntryError::Id)?;
    ledger
        .create_risk_from_reservation(
            &reserved,
            entry.title,
            entry.details,
            entry.classification,
            context,
            audit_event_id,
            now,
        )
        .map_err(RecordEntryError::from)
}
