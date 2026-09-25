//! The stable safe error envelope every IPC command returns (DG3 Error
//! Contract): `errorCode`, localization `messageKey`, safe typed
//! `messageParams`, `correlationId`, `retryable`, an optional
//! protected-local `privateDetailRef`, and typed extensions.
//!
//! One mapper for every failure class the host can see -- a domain error,
//! a Ledger transaction failure, a Ledger open failure, or a host-level
//! condition -- so no command invents its own shape and nothing that is
//! not in the domain's safe envelope (SQLite rows, paths, diagnostics)
//! can cross the boundary: the only inputs are the domain's already-safe
//! fields and host-chosen stable codes.
//!
//! Host-originated failures carry a host-minted correlation id (see
//! `runtime`), because a read command has no caller correlation and a
//! bare error with no id cannot be traced from the O05 copy button.

use pmc_domain::error::{DomainError, SafeErrorExtension, SafeParamValue};
use pmc_domain::identity::CorrelationId;
use pmc_ledger::sqlite::{LedgerOpenError, LedgerTransactionError};
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeErrorDto {
    pub error_code: String,
    pub message_key: String,
    pub message_params: Vec<MessageParamDto>,
    pub correlation_id: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_detail_ref: Option<String>,
    pub extensions: Vec<SafeErrorExtensionDto>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageParamDto {
    pub key: String,
    pub value: SafeParamValueDto,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum SafeParamValueDto {
    Identifier(String),
    FieldKey(String),
    Unsigned(u64),
    Boolean(bool),
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SafeErrorExtensionDto {
    CurrentVersion { version: u64 },
    FieldErrors { errors: Vec<FieldErrorDto> },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldErrorDto {
    pub field_key: String,
    pub reason_key: String,
}

impl SafeErrorDto {
    /// A host-level condition with a stable code and message key. The key
    /// is host vocabulary (`desktop.*`, `ledger.open.*`), localized by the
    /// same catalog as domain keys.
    pub fn host(
        error_code: &'static str,
        message_key: &'static str,
        correlation_id: &CorrelationId,
        retryable: bool,
    ) -> Self {
        Self {
            error_code: error_code.to_owned(),
            message_key: message_key.to_owned(),
            message_params: Vec::new(),
            correlation_id: correlation_id.as_str().to_owned(),
            retryable,
            private_detail_ref: None,
            extensions: Vec::new(),
        }
    }

    /// Lossless mapping of the domain's safe envelope. Nothing is added and
    /// nothing is dropped: every field the domain judged safe is carried.
    /// Production caller: the desktop write commands.
    pub fn from_domain(error: &DomainError) -> Self {
        Self {
            error_code: error.code().as_str().to_owned(),
            message_key: error.message_key().as_str().to_owned(),
            message_params: error
                .params()
                .iter()
                .map(|param| MessageParamDto {
                    key: param.key().to_owned(),
                    value: match param.value() {
                        SafeParamValue::Identifier(value) => {
                            SafeParamValueDto::Identifier(value.clone())
                        }
                        SafeParamValue::FieldKey(value) => {
                            SafeParamValueDto::FieldKey(value.clone())
                        }
                        SafeParamValue::Unsigned(value) => SafeParamValueDto::Unsigned(*value),
                        SafeParamValue::Boolean(value) => SafeParamValueDto::Boolean(*value),
                    },
                })
                .collect(),
            correlation_id: error.correlation_id().as_str().to_owned(),
            retryable: error.retryable(),
            private_detail_ref: error
                .private_detail_ref()
                .map(|reference| reference.as_str().to_owned()),
            extensions: error
                .extensions()
                .iter()
                .map(|extension| match extension {
                    SafeErrorExtension::CurrentVersion(version) => {
                        SafeErrorExtensionDto::CurrentVersion {
                            version: version.get(),
                        }
                    }
                    SafeErrorExtension::FieldErrors(errors) => SafeErrorExtensionDto::FieldErrors {
                        errors: errors
                            .iter()
                            .map(|field| FieldErrorDto {
                                field_key: field.field_key().to_owned(),
                                reason_key: field.reason_key().to_owned(),
                            })
                            .collect(),
                    },
                })
                .collect(),
        }
    }

    /// A Ledger transaction failure. The `Operation` variant carries the
    /// domain's own envelope; the kernel variants have no correlation of
    /// their own, so the caller's (or the host's) is attached.
    /// Production caller: the desktop write commands.
    pub fn from_transaction(
        error: LedgerTransactionError<DomainError>,
        correlation_id: &CorrelationId,
    ) -> Self {
        match error {
            LedgerTransactionError::Operation(domain) => Self::from_domain(&domain),
            // The record moved under the caller: re-read, then prepare again.
            LedgerTransactionError::RevisionConflict => Self::host(
                "DOMAIN_CONFLICT",
                "ledger.transaction.revision_conflict",
                correlation_id,
                false,
            ),
            // Non-retryable here: the fix is in System Health, not a retry.
            LedgerTransactionError::IncompatibleLedger => Self::host(
                "PLATFORM_INTERNAL",
                "ledger.transaction.incompatible_ledger",
                correlation_id,
                false,
            ),
            // Retryable with the SAME idempotency key.
            LedgerTransactionError::Busy => Self::host(
                "PLATFORM_INTERNAL",
                "ledger.transaction.busy",
                correlation_id,
                true,
            ),
            // The commit did not complete; only an exact replay under the
            // same idempotency key can say what happened.
            LedgerTransactionError::CommitFailed => Self::host(
                "PLATFORM_INTERNAL",
                "ledger.transaction.commit_failed",
                correlation_id,
                true,
            ),
        }
    }

    /// A Ledger open failure, with the same stable codes the host used
    /// before this envelope existed.
    pub fn from_open(error: LedgerOpenError, correlation_id: &CorrelationId) -> Self {
        let (code, key, retryable) = match error {
            LedgerOpenError::UnclaimedDatabase => (
                "LEDGER_UNCLAIMED_DATABASE",
                "ledger.open.unclaimed_database",
                false,
            ),
            LedgerOpenError::WrongApplication => (
                "LEDGER_WRONG_APPLICATION",
                "ledger.open.wrong_application",
                false,
            ),
            LedgerOpenError::FutureSchema { .. } => {
                ("LEDGER_FUTURE_SCHEMA", "ledger.open.future_schema", false)
            }
            LedgerOpenError::UnsupportedSchema { .. } => (
                "LEDGER_UNSUPPORTED_SCHEMA",
                "ledger.open.unsupported_schema",
                false,
            ),
            LedgerOpenError::InvalidMetadata => (
                "LEDGER_INVALID_METADATA",
                "ledger.open.invalid_metadata",
                false,
            ),
            LedgerOpenError::CorruptDatabase => (
                "LEDGER_CORRUPT_DATABASE",
                "ledger.open.corrupt_database",
                false,
            ),
            LedgerOpenError::PolicyViolation => (
                "LEDGER_POLICY_VIOLATION",
                "ledger.open.policy_violation",
                false,
            ),
            LedgerOpenError::Busy => ("LEDGER_BUSY", "ledger.open.busy", true),
            LedgerOpenError::StorageUnavailable => (
                "LEDGER_STORAGE_UNAVAILABLE",
                "ledger.open.storage_unavailable",
                true,
            ),
        };
        Self::host(code, key, correlation_id, retryable)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pmc_domain::error::{ErrorCode, FieldError, MessageKey, MessageParam, PrivateDetailRef};
    use pmc_domain::identity::AggregateVersion;

    fn correlation() -> CorrelationId {
        CorrelationId::parse("synthetic-correlation-1").unwrap()
    }

    #[test]
    fn a_domain_error_maps_losslessly_including_params_extensions_and_private_ref() {
        let error = DomainError::new(
            ErrorCode::DomainConflict,
            MessageKey::parse("evidence.pin_fingerprint_already_pinned").unwrap(),
            correlation(),
            false,
        )
        .with_param(
            MessageParam::new(
                "evidence_id",
                SafeParamValue::Identifier("evidence-1".into()),
            )
            .unwrap(),
        )
        .with_param(MessageParam::new("attempts", SafeParamValue::Unsigned(2)).unwrap())
        .with_extension(SafeErrorExtension::CurrentVersion(
            AggregateVersion::new(7).unwrap(),
        ))
        .with_extension(SafeErrorExtension::FieldErrors(vec![FieldError::new(
            "title",
            "validation.too_long",
        )
        .unwrap()]))
        .with_private_detail(PrivateDetailRef::parse("local-detail-9").unwrap());

        let dto = SafeErrorDto::from_domain(&error);

        assert_eq!(dto.error_code, "DOMAIN_CONFLICT");
        assert_eq!(dto.message_key, "evidence.pin_fingerprint_already_pinned");
        assert_eq!(dto.correlation_id, "synthetic-correlation-1");
        assert!(!dto.retryable);
        assert_eq!(dto.private_detail_ref.as_deref(), Some("local-detail-9"));
        assert_eq!(
            dto.message_params,
            vec![
                MessageParamDto {
                    key: "evidence_id".into(),
                    value: SafeParamValueDto::Identifier("evidence-1".into()),
                },
                MessageParamDto {
                    key: "attempts".into(),
                    value: SafeParamValueDto::Unsigned(2),
                },
            ]
        );
        assert_eq!(
            dto.extensions,
            vec![
                SafeErrorExtensionDto::CurrentVersion { version: 7 },
                SafeErrorExtensionDto::FieldErrors {
                    errors: vec![FieldErrorDto {
                        field_key: "title".into(),
                        reason_key: "validation.too_long".into(),
                    }],
                },
            ]
        );
    }

    #[test]
    fn kernel_transaction_failures_get_the_caller_correlation_and_the_right_retryability() {
        let busy = SafeErrorDto::from_transaction(LedgerTransactionError::Busy, &correlation());
        assert_eq!(busy.message_key, "ledger.transaction.busy");
        assert!(busy.retryable);
        assert_eq!(busy.correlation_id, "synthetic-correlation-1");

        let conflict = SafeErrorDto::from_transaction(
            LedgerTransactionError::RevisionConflict,
            &correlation(),
        );
        assert_eq!(conflict.error_code, "DOMAIN_CONFLICT");
        assert!(!conflict.retryable);

        let incompatible = SafeErrorDto::from_transaction(
            LedgerTransactionError::IncompatibleLedger,
            &correlation(),
        );
        assert!(!incompatible.retryable);

        let domain = DomainError::new(
            ErrorCode::DomainNotFound,
            MessageKey::parse("action.not_found").unwrap(),
            CorrelationId::parse("caller-correlation").unwrap(),
            false,
        );
        let mapped = SafeErrorDto::from_transaction(
            LedgerTransactionError::Operation(domain),
            &correlation(),
        );
        assert_eq!(
            mapped.correlation_id, "caller-correlation",
            "the domain's own correlation wins"
        );
        assert_eq!(mapped.error_code, "DOMAIN_NOT_FOUND");
    }

    #[test]
    fn open_failures_keep_their_stable_codes_and_gain_keys() {
        let busy = SafeErrorDto::from_open(LedgerOpenError::Busy, &correlation());
        assert_eq!(busy.error_code, "LEDGER_BUSY");
        assert_eq!(busy.message_key, "ledger.open.busy");
        assert!(busy.retryable);
        let future =
            SafeErrorDto::from_open(LedgerOpenError::FutureSchema { found: 99 }, &correlation());
        assert_eq!(future.error_code, "LEDGER_FUTURE_SCHEMA");
        assert!(!future.retryable);
        assert!(
            future.message_params.is_empty(),
            "no schema number leaks as a param"
        );
    }
}
