use crate::value::{validate_token, DomainValueError, ValueErrorKind};

const MAX_PROVENANCE_REFERENCE_LENGTH: usize = 128;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ProvenanceReference(String);

impl ProvenanceReference {
    pub fn parse(value: impl Into<String>) -> Result<Self, DomainValueError> {
        let value = value.into();
        validate_token(&value, MAX_PROVENANCE_REFERENCE_LENGTH, &['-', '_', '.'])?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Provenance {
    UserEntered,
    AuthoritativeTransition(ProvenanceReference),
    SyntheticFixture(ProvenanceReference),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProvenanceKind {
    UserEntered,
    AuthoritativeTransition,
    SyntheticFixture,
}

impl Provenance {
    #[must_use]
    pub const fn kind(&self) -> ProvenanceKind {
        match self {
            Self::UserEntered => ProvenanceKind::UserEntered,
            Self::AuthoritativeTransition(_) => ProvenanceKind::AuthoritativeTransition,
            Self::SyntheticFixture(_) => ProvenanceKind::SyntheticFixture,
        }
    }

    #[must_use]
    pub const fn kind_persisted(&self) -> &'static str {
        match self.kind() {
            ProvenanceKind::UserEntered => "user_entered",
            ProvenanceKind::AuthoritativeTransition => "authoritative_transition",
            ProvenanceKind::SyntheticFixture => "synthetic_fixture",
        }
    }

    pub fn kind_from_persisted(value: &str) -> Result<ProvenanceKind, DomainValueError> {
        match value {
            "user_entered" => Ok(ProvenanceKind::UserEntered),
            "authoritative_transition" => Ok(ProvenanceKind::AuthoritativeTransition),
            "synthetic_fixture" => Ok(ProvenanceKind::SyntheticFixture),
            _ => Err(DomainValueError::new(ValueErrorKind::UnknownPersistedValue)),
        }
    }

    #[must_use]
    pub const fn reference(&self) -> Option<&ProvenanceReference> {
        match self {
            Self::UserEntered => None,
            Self::AuthoritativeTransition(reference) | Self::SyntheticFixture(reference) => {
                Some(reference)
            }
        }
    }
}
