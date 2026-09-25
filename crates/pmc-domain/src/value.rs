use std::error::Error;
use std::fmt::{self, Display, Formatter};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct BoundedText<const MAXIMUM_LENGTH: usize>(String);

impl<const MAXIMUM_LENGTH: usize> BoundedText<MAXIMUM_LENGTH> {
    pub fn parse(value: impl Into<String>) -> Result<Self, DomainValueError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(DomainValueError::new(ValueErrorKind::Empty));
        }
        if value.len() > MAXIMUM_LENGTH {
            return Err(DomainValueError::new(ValueErrorKind::TooLong));
        }
        if value.chars().any(char::is_control) {
            return Err(DomainValueError::new(ValueErrorKind::InvalidCharacter));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueErrorKind {
    Empty,
    TooLong,
    InvalidCharacter,
    UnknownPersistedValue,
    Zero,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DomainValueError {
    kind: ValueErrorKind,
}

impl DomainValueError {
    pub const fn new(kind: ValueErrorKind) -> Self {
        Self { kind }
    }

    #[must_use]
    pub const fn kind(&self) -> ValueErrorKind {
        self.kind
    }
}

impl Display for DomainValueError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ValueErrorKind::Empty => "value is required",
            ValueErrorKind::TooLong => "value exceeds its maximum length",
            ValueErrorKind::InvalidCharacter => "value contains an invalid character",
            ValueErrorKind::UnknownPersistedValue => "persisted value is not recognized",
            ValueErrorKind::Zero => "value must be nonzero",
        })
    }
}

impl Error for DomainValueError {}

pub fn validate_token(
    value: &str,
    maximum_length: usize,
    allowed_punctuation: &[char],
) -> Result<(), DomainValueError> {
    if value.is_empty() {
        return Err(DomainValueError::new(ValueErrorKind::Empty));
    }
    if value.len() > maximum_length {
        return Err(DomainValueError::new(ValueErrorKind::TooLong));
    }
    if !value.chars().all(|character| {
        character.is_ascii_alphanumeric() || allowed_punctuation.contains(&character)
    }) {
        return Err(DomainValueError::new(ValueErrorKind::InvalidCharacter));
    }
    Ok(())
}
