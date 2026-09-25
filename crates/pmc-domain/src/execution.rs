//! Domain-only execution contract for high-impact relationship removal.

use crate::audit::AuditActor;
use crate::classification::DataClassification;
use crate::identity::{
    AggregateVersion, ApprovalReceiptId, PreparedIntentId, RecoveryEvidenceId, RelationshipId,
};
use crate::relationships::{
    EndpointSnapshot, OperationContext, RelationshipKind, StakeholderRelationshipPurpose,
};
use crate::time::UtcTimestamp;
use crate::value::BoundedText;
use sha2::{Digest, Sha256};

pub const REMOVE_RELATIONSHIP_INTENT_TYPE: &str = "relationship.remove";
pub const REMOVE_RELATIONSHIP_INTENT_VERSION: u16 = 1;
pub const MAX_PREPARED_INTENT_TTL_MILLIS: i64 = 300_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CancellationPolicy {
    NotCancellableAfterSubmit,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemovalPolicyDecision {
    Allowed,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemovalEffect {
    RemoveRelationshipRecord,
    RemoveSemanticRelationshipIndex,
    CreateIdempotencyTombstone,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryEvidence {
    id: RecoveryEvidenceId,
    name: BoundedText<128>,
    verified_at: UtcTimestamp,
    relationship_id: RelationshipId,
    compatible: bool,
}
impl RecoveryEvidence {
    pub fn new(
        id: RecoveryEvidenceId,
        name: impl Into<String>,
        verified_at: UtcTimestamp,
        relationship_id: RelationshipId,
        compatible: bool,
    ) -> Result<Self, crate::value::DomainValueError> {
        Ok(Self {
            id,
            name: BoundedText::parse(name.into())?,
            verified_at,
            relationship_id,
            compatible,
        })
    }
    pub fn id(&self) -> &RecoveryEvidenceId {
        &self.id
    }
    pub fn name(&self) -> &str {
        self.name.as_str()
    }
    pub const fn verified_at(&self) -> UtcTimestamp {
        self.verified_at
    }
    pub fn relationship_id(&self) -> &RelationshipId {
        &self.relationship_id
    }
    pub const fn compatible(&self) -> bool {
        self.compatible
    }
}
pub trait RecoveryEvidencePort {
    fn recovery_evidence(&self, relationship_id: &RelationshipId) -> Option<RecoveryEvidence>;
}
pub trait RemovalPolicyPort {
    fn allow_relationship_removal(
        &self,
        relationship_id: &RelationshipId,
        classification: DataClassification,
    ) -> bool;
}
pub trait ApprovalAuthorizationPort {
    fn authorize_relationship_removal(&self, actor: AuditActor) -> bool;
}
#[derive(Clone, Copy, Debug, Default)]
pub struct DenyApprovalAuthorization;
impl ApprovalAuthorizationPort for DenyApprovalAuthorization {
    fn authorize_relationship_removal(&self, _: AuditActor) -> bool {
        false
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct DenyRelationshipRemoval;
impl RemovalPolicyPort for DenyRelationshipRemoval {
    fn allow_relationship_removal(&self, _: &RelationshipId, _: DataClassification) -> bool {
        false
    }
}

pub trait ExecutionIdSource {
    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<PreparedIntentId, crate::value::DomainValueError>;
    /// Opaque receipt ID used only inside one cloned-state transaction.
    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<ApprovalReceiptId, crate::value::DomainValueError>;
}
#[derive(Clone, Debug, Default)]
pub struct SequentialExecutionIdSource(u64);
impl ExecutionIdSource for SequentialExecutionIdSource {
    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<PreparedIntentId, crate::value::DomainValueError> {
        self.0 += 1;
        PreparedIntentId::parse(format!("prepared-intent-{}", self.0))
    }
    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<ApprovalReceiptId, crate::value::DomainValueError> {
        self.0 += 1;
        ApprovalReceiptId::parse(format!("approval-receipt-{}", self.0))
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct UnavailableRecoveryEvidence;
impl RecoveryEvidencePort for UnavailableRecoveryEvidence {
    fn recovery_evidence(&self, _: &RelationshipId) -> Option<RecoveryEvidence> {
        None
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayloadDigest(String);
impl PayloadDigest {
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn parse(value: impl Into<String>) -> Result<Self, crate::value::DomainValueError> {
        let value = value.into();
        if value.len() != 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(crate::value::DomainValueError::new(
                crate::value::ValueErrorKind::InvalidCharacter,
            ));
        }
        Ok(Self(value))
    }
    pub(crate) fn for_preview(preview: &RemoveRelationshipPreview) -> Self {
        let mut h = Sha256::new();
        fn field(h: &mut Sha256, v: &[u8]) {
            h.update((v.len() as u64).to_be_bytes());
            h.update(v);
        }
        field(&mut h, preview.intent_type.as_bytes());
        field(&mut h, &preview.intent_version.to_be_bytes());
        field(&mut h, preview.prepared_id.to_string().as_bytes());
        field(&mut h, preview.relationship_id.to_string().as_bytes());
        field(&mut h, &preview.relationship_version.get().to_be_bytes());
        for endpoint in &preview.endpoints {
            for endpoint_field in endpoint.removal_digest_fields() {
                field(&mut h, endpoint_field.as_bytes());
            }
        }
        field(&mut h, preview.kind.as_persisted().as_bytes());
        field(&mut h, purpose_name(preview.purpose).as_bytes());
        for effect in &preview.effects {
            field(&mut h, effect_name(*effect).as_bytes());
        }
        field(&mut h, preview.classification.as_persisted().as_bytes());
        field(&mut h, b"allowed");
        field(&mut h, preview.evidence.id().to_string().as_bytes());
        field(&mut h, preview.evidence.name().as_bytes());
        field(
            &mut h,
            &preview.evidence.verified_at().unix_millis().to_be_bytes(),
        );
        field(
            &mut h,
            preview.evidence.relationship_id().to_string().as_bytes(),
        );
        field(&mut h, &[u8::from(preview.evidence.compatible())]);
        field(&mut h, &preview.expires_at.unix_millis().to_be_bytes());
        field(&mut h, b"not_cancellable_after_submit");
        field(&mut h, preview.confirmation_challenge.as_bytes());
        Self(format!("{:x}", h.finalize()))
    }
    pub(crate) fn for_confirmation(confirmation: &str) -> Self {
        let mut h = Sha256::new();
        h.update(b"relationship.remove.confirmation.v1");
        h.update((confirmation.len() as u64).to_be_bytes());
        h.update(confirmation.as_bytes());
        Self(format!("{:x}", h.finalize()))
    }
}
fn purpose_name(v: Option<StakeholderRelationshipPurpose>) -> &'static str {
    match v {
        None => "none",
        Some(StakeholderRelationshipPurpose::Responsibility) => "responsibility",
        Some(StakeholderRelationshipPurpose::Dependency) => "dependency",
    }
}
fn effect_name(v: RemovalEffect) -> &'static str {
    match v {
        RemovalEffect::RemoveRelationshipRecord => "remove_relationship_record",
        RemovalEffect::RemoveSemanticRelationshipIndex => "remove_semantic_relationship_index",
        RemovalEffect::CreateIdempotencyTombstone => "create_idempotency_tombstone",
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoveRelationshipPreview {
    pub(crate) intent_type: &'static str,
    pub(crate) intent_version: u16,
    pub(crate) prepared_id: PreparedIntentId,
    pub(crate) relationship_id: RelationshipId,
    pub(crate) relationship_version: AggregateVersion,
    pub(crate) endpoints: Vec<EndpointSnapshot>,
    pub(crate) kind: RelationshipKind,
    pub(crate) purpose: Option<StakeholderRelationshipPurpose>,
    pub(crate) effects: Vec<RemovalEffect>,
    pub(crate) classification: DataClassification,
    pub(crate) policy_decision: RemovalPolicyDecision,
    pub(crate) evidence: RecoveryEvidence,
    pub(crate) expires_at: UtcTimestamp,
    pub(crate) cancellation_policy: CancellationPolicy,
    pub(crate) confirmation_challenge: String,
}
impl RemoveRelationshipPreview {
    /// Persistence-adapter constructor. Callers must pass the resulting value
    /// through the owning Relationship snapshot validator before it can become
    /// authoritative state.
    #[doc(hidden)]
    #[allow(clippy::too_many_arguments)]
    pub fn from_persistence(
        prepared_id: PreparedIntentId,
        relationship_id: RelationshipId,
        relationship_version: AggregateVersion,
        endpoints: Vec<EndpointSnapshot>,
        kind: RelationshipKind,
        purpose: Option<StakeholderRelationshipPurpose>,
        effects: Vec<RemovalEffect>,
        classification: DataClassification,
        policy_decision: RemovalPolicyDecision,
        evidence: RecoveryEvidence,
        expires_at: UtcTimestamp,
        cancellation_policy: CancellationPolicy,
        confirmation_challenge: String,
    ) -> Self {
        Self {
            intent_type: REMOVE_RELATIONSHIP_INTENT_TYPE,
            intent_version: REMOVE_RELATIONSHIP_INTENT_VERSION,
            prepared_id,
            relationship_id,
            relationship_version,
            endpoints,
            kind,
            purpose,
            effects,
            classification,
            policy_decision,
            evidence,
            expires_at,
            cancellation_policy,
            confirmation_challenge,
        }
    }
    pub fn intent_type(&self) -> &str {
        self.intent_type
    }
    pub const fn intent_version(&self) -> u16 {
        self.intent_version
    }
    pub fn prepared_id(&self) -> &PreparedIntentId {
        &self.prepared_id
    }
    pub fn relationship_id(&self) -> &RelationshipId {
        &self.relationship_id
    }
    pub const fn relationship_version(&self) -> AggregateVersion {
        self.relationship_version
    }
    pub fn endpoints(&self) -> &[EndpointSnapshot] {
        &self.endpoints
    }
    pub const fn kind(&self) -> RelationshipKind {
        self.kind
    }
    pub const fn purpose(&self) -> Option<StakeholderRelationshipPurpose> {
        self.purpose
    }
    pub fn effects(&self) -> &[RemovalEffect] {
        &self.effects
    }
    pub const fn classification(&self) -> DataClassification {
        self.classification
    }
    pub const fn policy_decision(&self) -> RemovalPolicyDecision {
        self.policy_decision
    }
    pub fn evidence(&self) -> &RecoveryEvidence {
        &self.evidence
    }
    pub const fn expires_at(&self) -> UtcTimestamp {
        self.expires_at
    }
    pub const fn cancellation_policy(&self) -> CancellationPolicy {
        self.cancellation_policy
    }
    pub fn confirmation_challenge(&self) -> &str {
        &self.confirmation_challenge
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareRemoveRelationship {
    pub relationship_id: RelationshipId,
    pub context: OperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedIntent {
    pub(crate) preview: RemoveRelationshipPreview,
    pub(crate) payload_digest: PayloadDigest,
}
impl PreparedIntent {
    /// Persistence-adapter helper for reconstructing a typed candidate from
    /// normalized fields. The owning Relationship snapshot validator still
    /// verifies the preview against authoritative history before rehydration.
    #[doc(hidden)]
    pub fn from_persistence_preview(preview: RemoveRelationshipPreview) -> Self {
        let payload_digest = PayloadDigest::for_preview(&preview);
        Self {
            preview,
            payload_digest,
        }
    }

    /// Persistence-adapter constructor. The Relationship snapshot validator
    /// recomputes and checks the digest before rehydration.
    #[doc(hidden)]
    pub fn from_persistence(
        preview: RemoveRelationshipPreview,
        payload_digest: PayloadDigest,
    ) -> Self {
        Self {
            preview,
            payload_digest,
        }
    }
    pub fn id(&self) -> &PreparedIntentId {
        self.preview.prepared_id()
    }
    pub fn relationship_id(&self) -> &RelationshipId {
        self.preview.relationship_id()
    }
    pub fn preview(&self) -> &RemoveRelationshipPreview {
        &self.preview
    }
    pub fn confirmation_challenge(&self) -> &str {
        self.preview.confirmation_challenge()
    }
    pub fn payload_digest(&self) -> &PayloadDigest {
        &self.payload_digest
    }
    pub const fn expires_at(&self) -> UtcTimestamp {
        self.preview.expires_at()
    }
    pub const fn cancellation_policy(&self) -> CancellationPolicy {
        self.preview.cancellation_policy()
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproveAndExecuteRemoveRelationship {
    pub prepared_id: PreparedIntentId,
    pub actor: AuditActor,
    pub confirmation: String,
    pub acknowledged_payload_digest: PayloadDigest,
    pub context: OperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemovalOutcome {
    pub relationship_id: RelationshipId,
    pub audit_event_ids: Vec<crate::identity::AuditEventId>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::identity::{MilestoneId, PortfolioId, ProductId, ProjectId};
    use crate::relationships::{MilestoneSnapshot, PortfolioSnapshot, ProductSnapshot};

    fn preview(endpoints: Vec<EndpointSnapshot>) -> RemoveRelationshipPreview {
        let relationship_id = RelationshipId::parse("relationship-digest").unwrap();
        RemoveRelationshipPreview {
            intent_type: REMOVE_RELATIONSHIP_INTENT_TYPE,
            intent_version: REMOVE_RELATIONSHIP_INTENT_VERSION,
            prepared_id: PreparedIntentId::parse("prepared-digest").unwrap(),
            relationship_id: relationship_id.clone(),
            relationship_version: AggregateVersion::initial(),
            endpoints,
            kind: RelationshipKind::PortfolioProduct,
            purpose: None,
            effects: vec![RemovalEffect::RemoveRelationshipRecord],
            classification: DataClassification::Restricted,
            policy_decision: RemovalPolicyDecision::Allowed,
            evidence: RecoveryEvidence::new(
                RecoveryEvidenceId::parse("recovery-digest").unwrap(),
                "synthetic",
                UtcTimestamp::from_unix_millis(1),
                relationship_id,
                true,
            )
            .unwrap(),
            expires_at: UtcTimestamp::from_unix_millis(301_000),
            cancellation_policy: CancellationPolicy::NotCancellableAfterSubmit,
            confirmation_challenge: "REMOVE prepared-digest".into(),
        }
    }

    #[test]
    fn endpoint_classification_is_part_of_preview_digest_even_when_combined_value_is_same() {
        let restricted = EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
            PortfolioId::parse("portfolio-digest").unwrap(),
            AggregateVersion::initial(),
            DataClassification::Restricted,
        ));
        let public_product = EndpointSnapshot::Product(ProductSnapshot::new(
            ProductId::parse("product-digest").unwrap(),
            AggregateVersion::initial(),
            DataClassification::Public,
        ));
        let internal_product = EndpointSnapshot::Product(ProductSnapshot::new(
            ProductId::parse("product-digest").unwrap(),
            AggregateVersion::initial(),
            DataClassification::Internal,
        ));
        assert_ne!(
            PayloadDigest::for_preview(&preview(vec![restricted.clone(), public_product])),
            PayloadDigest::for_preview(&preview(vec![restricted, internal_product]))
        );
    }

    #[test]
    fn milestone_parent_project_is_part_of_preview_digest() {
        let milestone_id = MilestoneId::parse("milestone-digest").unwrap();
        let first = EndpointSnapshot::Milestone(MilestoneSnapshot::new(
            milestone_id.clone(),
            ProjectId::parse("project-one").unwrap(),
            AggregateVersion::initial(),
            DataClassification::Internal,
        ));
        let second = EndpointSnapshot::Milestone(MilestoneSnapshot::new(
            milestone_id,
            ProjectId::parse("project-two").unwrap(),
            AggregateVersion::initial(),
            DataClassification::Internal,
        ));
        assert_ne!(
            PayloadDigest::for_preview(&preview(vec![first])),
            PayloadDigest::for_preview(&preview(vec![second]))
        );
    }
}
