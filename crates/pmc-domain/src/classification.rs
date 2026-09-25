#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum DataClassification {
    Public,
    Internal,
    Confidential,
    Restricted,
    #[default]
    Unclassified,
}

impl DataClassification {
    #[must_use]
    pub const fn as_persisted(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Internal => "internal",
            Self::Confidential => "confidential",
            Self::Restricted => "restricted",
            Self::Unclassified => "unclassified",
        }
    }

    pub fn from_persisted(value: &str) -> Result<Self, DomainValueError> {
        match value {
            "public" => Ok(Self::Public),
            "internal" => Ok(Self::Internal),
            "confidential" => Ok(Self::Confidential),
            "restricted" => Ok(Self::Restricted),
            "unclassified" => Ok(Self::Unclassified),
            _ => Err(DomainValueError::new(ValueErrorKind::UnknownPersistedValue)),
        }
    }

    #[must_use]
    pub const fn combine(self, other: Self) -> Self {
        if matches!(self, Self::Unclassified) || matches!(other, Self::Unclassified) {
            return Self::Unclassified;
        }
        if self.restriction_rank() >= other.restriction_rank() {
            self
        } else {
            other
        }
    }

    const fn restriction_rank(self) -> u8 {
        match self {
            Self::Public => 0,
            Self::Internal => 1,
            Self::Confidential => 2,
            Self::Restricted => 3,
            Self::Unclassified => 4,
        }
    }
}
use crate::value::{DomainValueError, ValueErrorKind};
