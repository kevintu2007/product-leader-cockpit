//! Narrow local composition root for the shared Risk-to-Issue authority.
#![allow(clippy::result_large_err)]
//!
//! The raw authority is intentionally crate-private.  Consumers can construct
//! and stage the paired services only through this type, which prevents a
//! Risk service and an Issue service from accidentally using different stores.

use crate::issues::*;
use crate::risks::*;
use crate::time::Clock;
use crate::work_management::ApprovalAuthorizationPort;

pub struct WorkManagementRuntimeComposition<C, RI, II, Z, RP, RE, RC, IP, IE, IC> {
    risks: InMemoryRiskService<C, RI, Z, RP, RE, RC>,
    issues: InMemoryIssueService<C, II, Z, IP, IE, IC>,
    authority: SharedIssueAuthority,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkManagementRuntimeRehydrationError {
    InvalidIssueSnapshot,
    InvalidRiskSnapshot,
}

impl<C, RI, II, Z, RP, RE, RC, IP, IE, IC>
    WorkManagementRuntimeComposition<C, RI, II, Z, RP, RE, RC, IP, IE, IC>
where
    C: Clock,
    RI: RiskServiceIdSource,
    II: IssueServiceIdSource,
    Z: ApprovalAuthorizationPort,
    RP: RiskExecutionPolicyPort,
    RE: RiskEvidenceAuthorityPort,
    RC: RiskClassificationAuthorityPort,
    IP: IssueExecutionPolicyPort,
    IE: IssueEvidenceAuthorityPort,
    IC: IssueClassificationAuthorityPort,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        risk_clock: C,
        risk_ids: RI,
        risk_authorization: Z,
        risk_policy: RP,
        risk_evidence: RE,
        risk_classification: RC,
        issue_clock: C,
        issue_ids: II,
        issue_authorization: Z,
        issue_policy: IP,
        issue_evidence: IE,
        issue_classification: IC,
    ) -> Self {
        let authority = SharedIssueAuthority::new();
        Self {
            risks: InMemoryRiskService::new_with_issue_authority(
                risk_clock,
                risk_ids,
                risk_authorization,
                risk_policy,
                risk_evidence,
                risk_classification,
                authority.clone(),
            ),
            issues: InMemoryIssueService::new_with_issue_authority(
                issue_clock,
                issue_ids,
                issue_authorization,
                issue_policy,
                issue_evidence,
                issue_classification,
                authority.clone(),
            ),
            authority,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn rehydrate_with_h2a_and_issue_h1(
        risk_clock: C,
        risk_ids: RI,
        risk_authorization: Z,
        risk_policy: RP,
        risk_evidence: RE,
        risk_classification: RC,
        issue_clock: C,
        issue_ids: II,
        issue_authorization: Z,
        issue_policy: IP,
        issue_evidence: IE,
        issue_classification: IC,
        risk_snapshot: RiskH2aRuntimeSnapshot,
        issue_snapshot: IssueH1RuntimeSnapshot,
    ) -> Result<Self, WorkManagementRuntimeRehydrationError> {
        let mut issue_store = IssueStore::from_records(issue_snapshot.records().to_vec())
            .ok_or(WorkManagementRuntimeRehydrationError::InvalidIssueSnapshot)?;
        risk_snapshot
            .merge_occurrence_issues_into(&mut issue_store)
            .map_err(|_| WorkManagementRuntimeRehydrationError::InvalidRiskSnapshot)?;
        let authority = SharedIssueAuthority::new();
        authority.apply_unchecked(issue_store);
        let risks = InMemoryRiskService::rehydrate_with_h2a_and_issue_authority(
            risk_clock,
            risk_ids,
            risk_authorization,
            risk_policy,
            risk_evidence,
            risk_classification,
            risk_snapshot,
            authority.clone(),
        )
        .map_err(|_| WorkManagementRuntimeRehydrationError::InvalidRiskSnapshot)?;
        let issues = InMemoryIssueService::new_with_issue_authority(
            issue_clock,
            issue_ids,
            issue_authorization,
            issue_policy,
            issue_evidence,
            issue_classification,
            authority.clone(),
        );
        Ok(Self {
            risks,
            issues,
            authority,
        })
    }

    /// Reconstructs the shared runtime authority from a complete Risk H2a
    /// snapshot and a complete Issue H2a snapshot: every Issue at its current
    /// lifecycle state, whatever that state is.
    ///
    /// This is what a Risk occurrence needs. [`Self::rehydrate_with_h2a_and_issue_h1`]
    /// seeds the identity authority from standalone Issues *at their create
    /// state* and then merges the Risk namespace's own copy of its occurrence
    /// Issues; any standalone Issue that has since been resolved, closed,
    /// reopened or had its classification lowered is silently absent, and the
    /// duplicate-identity guard cannot see it. Here the records are complete,
    /// so nothing is merged: a Risk-derived Issue is already present at its
    /// current state, which the Risk namespace's copy can only approximate.
    #[allow(clippy::too_many_arguments)]
    pub fn rehydrate_with_risk_h2a_and_issue_h2a(
        risk_clock: C,
        risk_ids: RI,
        risk_authorization: Z,
        risk_policy: RP,
        risk_evidence: RE,
        risk_classification: RC,
        issue_clock: C,
        issue_ids: II,
        issue_authorization: Z,
        issue_policy: IP,
        issue_evidence: IE,
        issue_classification: IC,
        risk_snapshot: RiskH2aRuntimeSnapshot,
        issue_snapshot: IssueH2aRuntimeSnapshot,
    ) -> Result<Self, WorkManagementRuntimeRehydrationError> {
        let issue_store = IssueStore::from_records(issue_snapshot.records().to_vec())
            .ok_or(WorkManagementRuntimeRehydrationError::InvalidIssueSnapshot)?;
        let authority = SharedIssueAuthority::new();
        authority.apply_unchecked(issue_store);
        let risks = InMemoryRiskService::rehydrate_with_h2a_and_issue_authority(
            risk_clock,
            risk_ids,
            risk_authorization,
            risk_policy,
            risk_evidence,
            risk_classification,
            risk_snapshot,
            authority.clone(),
        )
        .map_err(|_| WorkManagementRuntimeRehydrationError::InvalidRiskSnapshot)?;
        let issues = InMemoryIssueService::rehydrate_with_prepared_and_issue_authority(
            issue_clock,
            issue_ids,
            issue_authorization,
            issue_policy,
            issue_evidence,
            issue_classification,
            issue_snapshot.prepared().to_vec(),
            issue_snapshot.rejections().to_vec(),
            authority.clone(),
        );
        Ok(Self {
            risks,
            issues,
            authority,
        })
    }

    /// Reconstructs the shared runtime authority from durable Issue H2a
    /// state alone: every Issue in its current lifecycle state, plus the
    /// specific outstanding prepared intent an
    /// `approve_and_execute_resolve/close/reopen` call is about to consume.
    /// The Risk side stays auxiliary and unused, mirroring how
    /// [`Self::rehydrate_with_h2a_and_issue_h1`] keeps the Issue side
    /// auxiliary for a Risk occurrence/close execute.
    #[allow(clippy::too_many_arguments)]
    pub fn rehydrate_with_issue_h2a(
        risk_clock: C,
        risk_ids: RI,
        risk_authorization: Z,
        risk_policy: RP,
        risk_evidence: RE,
        risk_classification: RC,
        issue_clock: C,
        issue_ids: II,
        issue_authorization: Z,
        issue_policy: IP,
        issue_evidence: IE,
        issue_classification: IC,
        issue_snapshot: IssueH2aRuntimeSnapshot,
    ) -> Result<Self, WorkManagementRuntimeRehydrationError> {
        let issue_store = IssueStore::from_records(issue_snapshot.records().to_vec())
            .ok_or(WorkManagementRuntimeRehydrationError::InvalidIssueSnapshot)?;
        let authority = SharedIssueAuthority::new();
        authority.apply_unchecked(issue_store);
        let risks = InMemoryRiskService::new_with_issue_authority(
            risk_clock,
            risk_ids,
            risk_authorization,
            risk_policy,
            risk_evidence,
            risk_classification,
            authority.clone(),
        );
        let issues = InMemoryIssueService::rehydrate_with_prepared_and_issue_authority(
            issue_clock,
            issue_ids,
            issue_authorization,
            issue_policy,
            issue_evidence,
            issue_classification,
            issue_snapshot.prepared().to_vec(),
            issue_snapshot.rejections().to_vec(),
            authority.clone(),
        );
        Ok(Self {
            risks,
            issues,
            authority,
        })
    }

    pub fn risk(&self, id: &crate::identity::RiskId) -> Option<&RiskRecord> {
        self.risks.risk(id)
    }

    pub fn issue(&self, id: &crate::identity::IssueId) -> Option<IssueRecord> {
        self.issues.issue(id)
    }

    pub fn risk_audit_events(&self) -> &[crate::audit::AuditEvent] {
        self.risks.audit_events()
    }

    pub fn issue_audit_events(&self) -> &[crate::audit::AuditEvent] {
        self.issues.audit_events()
    }

    pub fn risk_issue_links(&self) -> &[RiskIssueLink] {
        self.risks.risk_issue_links()
    }

    pub fn risk_issue_link(
        &self,
        risk_id: &crate::identity::RiskId,
        issue_id: &crate::identity::IssueId,
    ) -> Option<&RiskIssueLink> {
        self.risks.risk_issue_link(risk_id, issue_id)
    }

    pub fn inject_next_risk_commit_failure(&mut self) {
        self.risks.inject_next_commit_failure();
    }

    pub fn create_risk(
        &mut self,
        command: CreateRisk,
    ) -> Result<RiskMutationOutcome<RiskRecord>, crate::error::DomainError> {
        self.risks.create_risk(command)
    }

    pub fn update_risk_response(
        &mut self,
        command: UpdateRiskResponse,
    ) -> Result<RiskMutationOutcome<RiskRecord>, crate::error::DomainError> {
        self.risks.update_risk_response(command)
    }

    pub fn prepare_record_risk_occurrence(
        &mut self,
        command: PrepareRecordRiskOccurrence,
    ) -> Result<crate::work_management::WorkManagementPreparedIntent, crate::error::DomainError>
    {
        self.risks.prepare_record_risk_occurrence(command)
    }

    pub fn prepare_close_risk(
        &mut self,
        command: PrepareCloseRisk,
    ) -> Result<crate::work_management::WorkManagementPreparedIntent, crate::error::DomainError>
    {
        self.risks.prepare_close_risk(command)
    }

    pub fn approve_and_execute_record_risk_occurrence(
        &mut self,
        command: ApproveAndExecuteRecordRiskOccurrence,
    ) -> Result<OccurredRiskOutcome, crate::error::DomainError> {
        self.risks
            .approve_and_execute_record_risk_occurrence(command)
    }

    /// Same execution as [`Self::approve_and_execute_record_risk_occurrence`],
    /// additionally reporting whether a failure is the one accepted durable
    /// post-start terminal a persistence adapter must reconstruct as an exact
    /// replay. See `InMemoryRiskService::approve_and_execute_record_risk_occurrence_with_durability`.
    pub fn approve_and_execute_record_risk_occurrence_with_durability(
        &mut self,
        command: ApproveAndExecuteRecordRiskOccurrence,
    ) -> (Result<OccurredRiskOutcome, crate::error::DomainError>, bool) {
        self.risks
            .approve_and_execute_record_risk_occurrence_with_durability(command)
    }

    pub fn approve_and_execute_close_risk(
        &mut self,
        command: ApproveAndExecuteCloseRisk,
    ) -> Result<RiskMutationOutcome<RiskRecord>, crate::error::DomainError> {
        self.risks.approve_and_execute_close_risk(command)
    }

    /// Same execution as [`Self::approve_and_execute_close_risk`], additionally
    /// reporting the durable-terminal signal. See
    /// [`Self::approve_and_execute_record_risk_occurrence_with_durability`].
    pub fn approve_and_execute_close_risk_with_durability(
        &mut self,
        command: ApproveAndExecuteCloseRisk,
    ) -> (
        Result<RiskMutationOutcome<RiskRecord>, crate::error::DomainError>,
        bool,
    ) {
        self.risks
            .approve_and_execute_close_risk_with_durability(command)
    }

    pub fn risk_reenters_queue(
        &self,
        id: &crate::identity::RiskId,
        now: crate::time::UtcTimestamp,
        exposure_increased: bool,
        control_invalid: bool,
    ) -> Result<bool, crate::error::DomainError> {
        self.risks
            .risk_reenters_queue(id, now, exposure_increased, control_invalid)
    }

    pub fn discard_risk_prepared_intent(&mut self, id: &crate::identity::PreparedIntentId) -> bool {
        self.risks.discard_prepared_intent(id)
    }

    /// v46: the recorded refusal of a pending Risk preview.
    pub fn reject_risk_prepared_intent(
        &mut self,
        command: crate::risks::RejectRiskPreparedIntent,
    ) -> Result<crate::work_management::RejectedPreparedIntentOutcome, crate::error::DomainError>
    {
        self.risks.reject_risk_prepared_intent(command)
    }

    pub fn create_issue(
        &mut self,
        command: CreateIssue,
    ) -> Result<IssueMutationOutcome, crate::error::DomainError> {
        self.issues.create_issue(command)
    }

    pub fn prepare_resolve_issue(
        &mut self,
        command: PrepareResolveIssue,
    ) -> Result<crate::work_management::WorkManagementPreparedIntent, crate::error::DomainError>
    {
        self.issues.prepare_resolve_issue(command)
    }

    pub fn prepare_close_issue(
        &mut self,
        command: PrepareCloseIssue,
    ) -> Result<crate::work_management::WorkManagementPreparedIntent, crate::error::DomainError>
    {
        self.issues.prepare_close_issue(command)
    }

    pub fn prepare_reopen_issue(
        &mut self,
        command: PrepareReopenIssue,
    ) -> Result<crate::work_management::WorkManagementPreparedIntent, crate::error::DomainError>
    {
        self.issues.prepare_reopen_issue(command)
    }

    pub fn approve_and_execute_resolve_issue(
        &mut self,
        command: ApproveAndExecuteIssueTransition,
    ) -> Result<IssueMutationOutcome, crate::error::DomainError> {
        self.issues.approve_and_execute_resolve(command)
    }

    pub fn approve_and_execute_close_issue(
        &mut self,
        command: ApproveAndExecuteIssueTransition,
    ) -> Result<IssueMutationOutcome, crate::error::DomainError> {
        self.issues.approve_and_execute_close(command)
    }

    pub fn approve_and_execute_reopen_issue(
        &mut self,
        command: ApproveAndExecuteIssueTransition,
    ) -> Result<IssueMutationOutcome, crate::error::DomainError> {
        self.issues.approve_and_execute_reopen(command)
    }

    /// H2a "Lower Data Classification" for Issue. Additive
    /// forwarding wrapper, mirroring `approve_and_execute_resolve_issue`'s
    /// exact shape: `InMemoryIssueService::approve_and_execute_lower_issue_
    /// classification` is already public, but its owning `issues` field is
    /// private, and this operation has no equivalent wrapper yet.
    pub fn approve_and_execute_lower_issue_classification(
        &mut self,
        command: ApproveAndExecuteLowerIssueClassification,
    ) -> Result<IssueMutationOutcome, crate::error::DomainError> {
        self.issues
            .approve_and_execute_lower_issue_classification(command)
    }

    pub fn discard_issue_prepared_intent(
        &mut self,
        id: &crate::identity::PreparedIntentId,
    ) -> bool {
        self.issues.discard_prepared_intent(id)
    }

    /// v46: the recorded refusal of a pending Issue preview.
    pub fn reject_issue_prepared_intent(
        &mut self,
        command: crate::issues::RejectIssuePreparedIntent,
    ) -> Result<crate::work_management::RejectedPreparedIntentOutcome, crate::error::DomainError>
    {
        self.issues.reject_issue_prepared_intent(command)
    }

    pub fn isolated_stage(&self) -> Self
    where
        C: Clone,
        RI: Clone,
        II: Clone,
        Z: Clone,
        RP: Clone,
        RE: Clone,
        RC: Clone,
        IP: Clone,
        IE: Clone,
        IC: Clone,
    {
        let authority = self.authority.isolated_copy();
        Self {
            risks: self
                .risks
                .isolated_copy_with_issue_authority(authority.clone()),
            issues: self
                .issues
                .isolated_copy_with_issue_authority(authority.clone()),
            authority,
        }
    }
}
