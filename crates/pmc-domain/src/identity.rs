use std::fmt::{self, Display, Formatter};

use crate::value::{validate_token, DomainValueError, ValueErrorKind};

const MAX_ENTITY_ID_LENGTH: usize = 64;
const MAX_OPERATION_ID_LENGTH: usize = 128;

macro_rules! opaque_id {
    ($name:ident, $maximum:expr) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, DomainValueError> {
                let value = value.into();
                validate_token(&value, $maximum, &['-', '_'])?;
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

opaque_id!(PortfolioId, MAX_ENTITY_ID_LENGTH);
opaque_id!(ProductId, MAX_ENTITY_ID_LENGTH);
opaque_id!(InitiativeId, MAX_ENTITY_ID_LENGTH);
opaque_id!(ProjectId, MAX_ENTITY_ID_LENGTH);
opaque_id!(RoadmapId, MAX_ENTITY_ID_LENGTH);
opaque_id!(MilestoneId, MAX_ENTITY_ID_LENGTH);
opaque_id!(KpiId, MAX_ENTITY_ID_LENGTH);
opaque_id!(KpiObservationId, MAX_ENTITY_ID_LENGTH);
opaque_id!(StakeholderId, MAX_ENTITY_ID_LENGTH);
opaque_id!(RelationshipId, MAX_ENTITY_ID_LENGTH);
opaque_id!(ActionRequestId, MAX_ENTITY_ID_LENGTH);
opaque_id!(ActionId, MAX_ENTITY_ID_LENGTH);
opaque_id!(DecisionRequestId, MAX_ENTITY_ID_LENGTH);
opaque_id!(DecisionId, MAX_ENTITY_ID_LENGTH);
opaque_id!(RiskId, MAX_ENTITY_ID_LENGTH);
opaque_id!(IssueId, MAX_ENTITY_ID_LENGTH);
opaque_id!(EvidenceReferenceId, MAX_ENTITY_ID_LENGTH);
opaque_id!(AuditEventId, MAX_ENTITY_ID_LENGTH);
opaque_id!(PreparedIntentId, MAX_OPERATION_ID_LENGTH);
opaque_id!(ApprovalReceiptId, MAX_OPERATION_ID_LENGTH);
opaque_id!(RecoveryEvidenceId, MAX_OPERATION_ID_LENGTH);
opaque_id!(OperationId, MAX_OPERATION_ID_LENGTH);
opaque_id!(CorrelationId, MAX_OPERATION_ID_LENGTH);
opaque_id!(IdempotencyId, MAX_OPERATION_ID_LENGTH);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AggregateVersion(u64);

impl AggregateVersion {
    pub const fn initial() -> Self {
        Self(1)
    }

    pub fn new(value: u64) -> Result<Self, DomainValueError> {
        if value == 0 {
            return Err(DomainValueError::new(ValueErrorKind::Zero));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    #[must_use]
    pub fn next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}
