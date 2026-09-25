use std::error::Error;
use std::fmt::{self, Display, Formatter};

use crate::identity::{AggregateVersion, CorrelationId};
use crate::value::{validate_token, DomainValueError};

const MAX_KEY_LENGTH: usize = 96;
const MAX_PRIVATE_DETAIL_REF_LENGTH: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorCode {
    ValidationInvalidField,
    DomainConflict,
    DomainNotFound,
    SecurityPolicyDenied,
    AiPolicyDenied,
    SecurityPreviewExpiredOrChanged,
    DomainIdempotencyConflict,
    PlatformInternal,
}

impl ErrorCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ValidationInvalidField => "VALIDATION_INVALID_FIELD",
            Self::DomainConflict => "DOMAIN_CONFLICT",
            Self::DomainNotFound => "DOMAIN_NOT_FOUND",
            Self::SecurityPolicyDenied => "SECURITY_POLICY_DENIED",
            Self::AiPolicyDenied => "AI_POLICY_DENIED",
            Self::SecurityPreviewExpiredOrChanged => "SECURITY_PREVIEW_EXPIRED_OR_CHANGED",
            Self::DomainIdempotencyConflict => "DOMAIN_IDEMPOTENCY_CONFLICT",
            Self::PlatformInternal => "PLATFORM_INTERNAL",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageKey(String);

impl MessageKey {
    pub fn parse(value: impl Into<String>) -> Result<Self, DomainValueError> {
        let value = value.into();
        validate_token(&value, MAX_KEY_LENGTH, &['-', '_', '.'])?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SafeParamValue {
    Identifier(String),
    FieldKey(String),
    Unsigned(u64),
    Boolean(bool),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageParam {
    key: String,
    value: SafeParamValue,
}

impl MessageParam {
    pub fn new(key: impl Into<String>, value: SafeParamValue) -> Result<Self, DomainValueError> {
        let key = key.into();
        validate_token(&key, MAX_KEY_LENGTH, &['-', '_', '.'])?;
        match &value {
            SafeParamValue::Identifier(value) => {
                validate_token(value, MAX_KEY_LENGTH, &['-', '_'])?;
            }
            SafeParamValue::FieldKey(value) => {
                validate_token(value, MAX_KEY_LENGTH, &['-', '_', '.'])?;
            }
            SafeParamValue::Unsigned(_) | SafeParamValue::Boolean(_) => {}
        }
        Ok(Self { key, value })
    }

    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    #[must_use]
    pub const fn value(&self) -> &SafeParamValue {
        &self.value
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldError {
    field_key: String,
    reason_key: String,
}

impl FieldError {
    pub fn new(
        field_key: impl Into<String>,
        reason_key: impl Into<String>,
    ) -> Result<Self, DomainValueError> {
        let field_key = field_key.into();
        let reason_key = reason_key.into();
        validate_token(&field_key, MAX_KEY_LENGTH, &['-', '_', '.'])?;
        validate_token(&reason_key, MAX_KEY_LENGTH, &['-', '_', '.'])?;
        Ok(Self {
            field_key,
            reason_key,
        })
    }

    #[must_use]
    pub fn field_key(&self) -> &str {
        &self.field_key
    }

    #[must_use]
    pub fn reason_key(&self) -> &str {
        &self.reason_key
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrivateDetailRef(String);

impl PrivateDetailRef {
    pub fn parse(value: impl Into<String>) -> Result<Self, DomainValueError> {
        let value = value.into();
        validate_token(&value, MAX_PRIVATE_DETAIL_REF_LENGTH, &['-', '_', '.'])?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SafeErrorExtension {
    CurrentVersion(AggregateVersion),
    FieldErrors(Vec<FieldError>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DomainError {
    code: ErrorCode,
    message_key: MessageKey,
    params: Vec<MessageParam>,
    correlation_id: CorrelationId,
    retryable: bool,
    extensions: Vec<SafeErrorExtension>,
    private_detail_ref: Option<PrivateDetailRef>,
}

impl DomainError {
    #[must_use]
    pub const fn new(
        code: ErrorCode,
        message_key: MessageKey,
        correlation_id: CorrelationId,
        retryable: bool,
    ) -> Self {
        Self {
            code,
            message_key,
            params: Vec::new(),
            correlation_id,
            retryable,
            extensions: Vec::new(),
            private_detail_ref: None,
        }
    }

    #[must_use]
    pub fn with_param(mut self, param: MessageParam) -> Self {
        self.params.push(param);
        self
    }

    #[must_use]
    pub fn with_extension(mut self, extension: SafeErrorExtension) -> Self {
        self.extensions.push(extension);
        self
    }

    #[must_use]
    pub fn with_private_detail(mut self, reference: PrivateDetailRef) -> Self {
        self.private_detail_ref = Some(reference);
        self
    }

    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        self.code
    }

    #[must_use]
    pub const fn message_key(&self) -> &MessageKey {
        &self.message_key
    }

    #[must_use]
    pub fn params(&self) -> &[MessageParam] {
        &self.params
    }

    #[must_use]
    pub const fn correlation_id(&self) -> &CorrelationId {
        &self.correlation_id
    }

    #[must_use]
    pub const fn retryable(&self) -> bool {
        self.retryable
    }

    #[must_use]
    pub fn extensions(&self) -> &[SafeErrorExtension] {
        &self.extensions
    }

    #[must_use]
    pub const fn private_detail_ref(&self) -> Option<&PrivateDetailRef> {
        self.private_detail_ref.as_ref()
    }
}

impl Display for DomainError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "domain operation failed ({})",
            self.code.as_str()
        )
    }
}

impl Error for DomainError {}
