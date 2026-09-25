//! The desktop's production adapters for the work-management H2a ports.
//!
//! Every H2a execution in `pmc-domain` is generic over an authorization
//! port and a policy port. The Ledger crate ships two kinds of adapter:
//! deny-everything placeholders that must never be reached, and permissive
//! ones inside `decision_repository` used to rehydrate persisted state.
//! Neither is a statement about who may approve what in the running app.
//! These are: named, explicit, and the only ones the desktop write path
//! hands to the Ledger.
//!
//! v1 is a single-user application (DG0): the one person operating it is
//! the Head of Products, and the domain already refuses any other actor at
//! `WorkManagementApproval::new`. The execution policy for work-management
//! H2a operations is `Allowed`: DG0's policy denials (kill-switch,
//! classification, external AI) live on other ports and are not folded in
//! here, so this adapter cannot be mistaken for them.

use pmc_domain::actions::{
    ActionEvidenceAuthorityError, ActionEvidenceAuthorityPort, ActionExecutionPolicy,
    ActionExecutionPolicyPort,
};
use pmc_domain::audit::AuditActor;
use pmc_domain::decisions::{DecisionExecutionPolicy, DecisionExecutionPolicyPort};
use pmc_domain::identity::EvidenceReferenceId;
use pmc_domain::issues::{IssueExecutionPolicy, IssueExecutionPolicyPort};
use pmc_domain::risks::{RiskExecutionPolicy, RiskExecutionPolicyPort};
use pmc_domain::work_management::{
    ApprovalAuthorizationPort, EvidenceReferenceMetadata, WorkManagementOperation,
};

/// Only the Head of Products approves. The domain enforces the same rule
/// when the approval is constructed; this adapter is the execution-time
/// re-check the ports require, and it agrees.
#[derive(Clone, Copy, Debug, Default)]
pub struct HeadOfProductsApproval;

impl ApprovalAuthorizationPort for HeadOfProductsApproval {
    fn authorize(&self, actor: AuditActor) -> bool {
        actor == AuditActor::HeadOfProducts
    }
}

/// The v1 single-user execution policy: work-management H2a operations are
/// allowed once approved. Preparation still gates on Evidence,
/// classification and versions; this port answers only "is this class of
/// operation executable for this user", and in v1 it is.
#[derive(Clone, Copy, Debug, Default)]
pub struct SingleUserExecutionPolicy;

impl ActionExecutionPolicyPort for SingleUserExecutionPolicy {
    fn current_policy(&self, _: &WorkManagementOperation) -> ActionExecutionPolicy {
        ActionExecutionPolicy::Allowed
    }
}

impl DecisionExecutionPolicyPort for SingleUserExecutionPolicy {
    fn current_policy(&self, _: &WorkManagementOperation) -> DecisionExecutionPolicy {
        DecisionExecutionPolicy::Allowed
    }
}

impl RiskExecutionPolicyPort for SingleUserExecutionPolicy {
    fn current_policy(&self, _: &WorkManagementOperation) -> RiskExecutionPolicy {
        RiskExecutionPolicy::Allowed
    }
}

impl IssueExecutionPolicyPort for SingleUserExecutionPolicy {
    fn current_policy(&self, _: &WorkManagementOperation) -> IssueExecutionPolicy {
        IssueExecutionPolicy::Allowed
    }
}

/// An evidence authority that resolves nothing. Preparing an Action Request
/// acceptance never consults Evidence, so the rehydrated domain service can
/// be given this honestly; a flow that does need Evidence (Action
/// completion) must be given the persisted authority instead, and this
/// adapter's `Unavailable` makes such a mistake fail closed rather than
/// pass silently.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoEvidenceAuthority;

impl ActionEvidenceAuthorityPort for NoEvidenceAuthority {
    fn resolve(
        &self,
        _: &EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, ActionEvidenceAuthorityError> {
        Err(ActionEvidenceAuthorityError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_head_of_products_is_authorized() {
        assert!(HeadOfProductsApproval.authorize(AuditActor::HeadOfProducts));
        assert!(!HeadOfProductsApproval.authorize(AuditActor::PolicyAuthorizedSystem));
    }

    #[test]
    fn the_evidence_placeholder_never_resolves() {
        let id = EvidenceReferenceId::parse("evidence-1").unwrap_or_else(|_| unreachable!());
        assert_eq!(
            NoEvidenceAuthority.resolve(&id),
            Err(ActionEvidenceAuthorityError::Unavailable)
        );
    }
}
