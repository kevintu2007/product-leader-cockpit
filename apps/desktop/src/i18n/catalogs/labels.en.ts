/**
 * English words for the identifiers the host sends about records: lifecycle
 * intents, states, attention reasons, ranking tiers and the rest.
 *
 * The host speaks in stable identifiers; those are contracts and must not be
 * reworded, and a person should never have to read them. These are the only
 * words for them. `workLabels.test.ts` reads the Rust sources, so an
 * identifier the host gains without a word here fails the build.
 *
 * Canonical domain nouns (Action Request, Decision Request, Risk, Issue,
 * Evidence, ...) and classification names stay in English in every language.
 */
export const LABELS_EN = {
  // Lifecycle intents.
  "label.intent.submit_action_request": "Submit request",
  "label.intent.prepare_accept_action_request": "Accept request",
  "label.intent.decline_action_request": "Decline",
  "label.intent.withdraw_action_request": "Withdraw",
  "label.intent.start_action": "Start",
  "label.intent.link_action_completion_evidence": "Attach completion Evidence",
  "label.intent.prepare_complete_action": "Mark complete",
  "label.intent.prepare_cancel_action": "Cancel",
  "label.intent.prepare_reopen_action": "Reopen",
  "label.intent.submit_decision_request": "Submit request",
  "label.intent.withdraw_decision_request": "Withdraw",
  "label.intent.prepare_resolve_decision_request": "Decide",
  "label.intent.prepare_record_risk_occurrence": "Record occurrence",
  "label.intent.prepare_close_risk": "Close Risk",
  "label.intent.prepare_resolve_issue": "Resolve",
  "label.intent.prepare_close_issue": "Close",
  "label.intent.prepare_reopen_issue": "Reopen",

  // States, as the Work Queue names them.
  "label.state.Draft": "Draft",
  "label.state.Open": "Open",
  "label.state.Accepted": "Accepted",
  "label.state.Declined": "Declined",
  "label.state.Withdrawn": "Withdrawn",
  "label.state.In progress": "In progress",
  "label.state.Completed": "Completed",
  "label.state.Cancelled": "Cancelled",
  "label.state.Resolved": "Resolved",
  "label.state.Occurred": "Occurred",
  "label.state.Closed": "Closed",

  // States, as write outcomes persist them.
  "label.persistedState.draft": "Draft",
  "label.persistedState.open": "Open",
  "label.persistedState.accepted": "Accepted",
  "label.persistedState.declined": "Declined",
  "label.persistedState.withdrawn": "Withdrawn",
  "label.persistedState.in_progress": "In progress",
  "label.persistedState.completed": "Completed",
  "label.persistedState.cancelled": "Cancelled",
  "label.persistedState.resolved": "Resolved",
  "label.persistedState.effective": "In effect",
  "label.persistedState.superseded": "Superseded",
  "label.persistedState.occurred": "Occurred",
  "label.persistedState.closed": "Closed",

  // Attention reasons: what happened, in one short clause.
  "label.reason.action_request_needs_info": "The request is missing information",
  "label.reason.action_request_stale": "The request has sat too long without progress",
  "label.reason.action_request_missing_intended_owner": "No owner has been named yet",
  "label.reason.action_request_response_due": "The response is due soon",
  "label.reason.action_request_response_overdue": "The response is overdue",
  "label.reason.action_blocked": "The work is blocked",
  "label.reason.action_overdue": "Past its due date",
  "label.reason.action_at_risk": "May not finish on time",
  "label.reason.action_needs_evidence": "Needs Evidence before it can complete",
  "label.reason.action_evidence_verification_pending": "Evidence is waiting to be verified",
  "label.reason.action_superseded_premise": "The Decision it rests on was superseded",
  "label.reason.decision_request_needs_info": "The decision request is missing information",
  "label.reason.decision_request_missing_decision_owner": "No decision maker has been named yet",
  "label.reason.decision_request_approaching_deadline": "The decision is due soon",
  "label.reason.decision_request_overdue": "The decision is overdue",
  "label.reason.decision_request_stale": "The decision request has sat too long",
  "label.reason.risk_review_due": "Time to review this Risk",
  "label.reason.risk_exposure_increased": "Exposure has increased",
  "label.reason.risk_evidence_stale": "The Risk's Evidence is out of date",
  "label.reason.risk_control_invalid": "A control for this Risk no longer works",
  "label.reason.risk_missing_owner": "The Risk has no owner",
  "label.reason.issue_blocked": "Work on the Issue is blocked",
  "label.reason.issue_resolution_due": "The resolution is due soon",
  "label.reason.issue_resolution_overdue": "The resolution is overdue",
  "label.reason.issue_needs_evidence": "Needs Evidence to proceed",
  "label.reason.issue_stale": "The Issue has sat too long",
  "label.reason.issue_recurrence": "The same Issue happened again",

  // Ranking tiers, and where an item was placed.
  "label.tier.breachedCommitment": "A commitment was missed",
  "label.tier.blocked": "Work can't move forward",
  "label.tier.evidenceIntegrity": "The Evidence it rests on has a problem",
  "label.tier.approachingDeadline": "A deadline is near",
  "label.tier.noAccountableOwner": "Nobody is accountable",
  "label.tier.other": "Worth a look",
  "label.placement.unflagged": "Nothing flagged it, so it sorts after flagged work",

  "label.freshness.fresh": "current",
  "label.freshness.stale": "may be out of date",
  "label.freshness.unknown": "can't confirm it is current",

  // The five Work Queue lifecycle types.
  "label.workItemKind.action_request": "Action Request",
  "label.workItemKind.action": "Action",
  "label.workItemKind.decision_request": "Decision Request",
  "label.workItemKind.risk": "Risk",
  "label.workItemKind.issue": "Issue",

  // Milestone timing, as the Lens axis names it. A passed date is never
  // called overdue: Milestones have no completion state.
  "label.timing.unknown": "No milestones",
  "label.timing.later": "Not yet due",
  "label.timing.dueSoon": "Due soon",
  "label.timing.datePassed": "Date passed",

  // The four accepted quadrant names.
  "label.quadrant.keepMomentum": "Keep momentum",
  "label.quadrant.monitorClosely": "Monitor closely",
  "label.quadrant.exploreAndValidate": "Explore and validate",
  "label.quadrant.prioritizeNow": "Prioritize now",

  // The host's fixed Cockpit sentences. The last continues a sentence the
  // screen starts ("can't compare: ..."), so it keeps the host's lower case.
  "label.cockpit.milestonesTracked": "Milestones currently tracked across the Portfolio",
  "label.cockpit.acceptedActions":
    "Accepted Actions. A submitted Action Request is not counted until it is accepted",
  "label.cockpit.kpisDefined": "KPIs with a definition in the Ledger",
  "label.cockpit.noApprovedPeriod":
    "no review period has been approved yet, so there is nothing to compare against",

  // The records a Lens measure stands on.
  "label.contribution.relationship": "Relationship",
  "label.contribution.project": "Project",
  "label.contribution.milestone": "Milestone",
  "label.contribution.kpi_definition": "KPI definition",
  "label.contribution.kpi_observation": "KPI observation",
  "label.contribution.evidence_link": "Evidence link",
  "label.contribution.evidence_reference": "Evidence",
  "label.relationshipKind.project_product": "Project–Product relationship",
  "label.relationshipKind.product_kpi": "Product–KPI relationship",

  // The kind of record a prepared change targets.
  "label.targetKind.action_request": "Action Request",
  "label.targetKind.action": "Action",
  "label.targetKind.decision_request": "Decision Request",
  "label.targetKind.decision": "Decision",
  "label.targetKind.risk": "Risk",
  "label.targetKind.issue": "Issue",
  "label.targetKind.portfolio": "Portfolio",
  "label.targetKind.product": "Product",
  "label.targetKind.roadmap": "Roadmap",
  "label.targetKind.kpi": "KPI",
  "label.targetKind.kpi_observation": "KPI observation",
  "label.targetKind.initiative": "Initiative",
  "label.targetKind.project": "Project",
  "label.targetKind.milestone": "Milestone",

  // What approving does, one effect at a time.
  "label.effect.accept_action_request": "Accept the Action Request",
  "label.effect.create_action": "Create an Action",
  "label.effect.link_action_request_to_action": "Link the Action Request to the new Action",
  "label.effect.resolve_decision_request": "Resolve the Decision Request",
  "label.effect.create_decision": "Create a Decision",
  "label.effect.link_decision_request_to_decision": "Link the Decision Request to the Decision",
  "label.effect.create_resulting_action_request": "Create a follow-up Action Request",
  "label.effect.link_decision_to_action_request":
    "Link the Decision to the follow-up Action Request",
  "label.effect.complete_action": "Mark the Action complete",
  "label.effect.cancel_action": "Cancel the Action",
  "label.effect.reopen_action": "Reopen the Action",
  "label.effect.supersede_decision": "Replace it with a new Decision",
  "label.effect.link_replacement_decision": "Link to the Decision that replaces it",
  "label.effect.flag_superseded_premise_action_request":
    "Flag that the Action Request's basis was replaced",
  "label.effect.flag_superseded_premise_action": "Flag that the Action's basis was replaced",
  "label.effect.record_risk_occurrence": "Record that the Risk occurred",
  "label.effect.create_issue": "Create an Issue",
  "label.effect.link_risk_to_issue": "Link the Risk to the Issue",
  "label.effect.close_risk": "Close the Risk",
  "label.effect.resolve_issue": "Resolve the Issue",
  "label.effect.close_issue": "Close the Issue",
  "label.effect.reopen_issue": "Reopen the Issue",
  "label.effect.lower_portfolio_classification": "Lower the Portfolio's classification",
  "label.effect.lower_product_classification": "Lower the Product's classification",
  "label.effect.lower_roadmap_classification": "Lower the Roadmap's classification",
  "label.effect.lower_kpi_classification": "Lower the KPI's classification",
  "label.effect.lower_kpi_observation_classification": "Lower the KPI observation's classification",
  "label.effect.lower_action_classification": "Lower the Action's classification",
  "label.effect.lower_decision_classification": "Lower the Decision's classification",
  "label.effect.lower_risk_classification": "Lower the Risk's classification",
  "label.effect.lower_issue_classification": "Lower the Issue's classification",
  "label.effect.lower_initiative_classification": "Lower the Initiative's classification",
  "label.effect.lower_project_classification": "Lower the Project's classification",
  "label.effect.lower_milestone_classification": "Lower the Milestone's classification",

  // Which record a classification was folded in from.
  "label.sourceRole.primary_target": "This record itself",
  "label.sourceRole.created_action": "The Action it will create",
  "label.sourceRole.created_decision": "The Decision it will create",
  "label.sourceRole.replacement_decision": "The replacement Decision",
  "label.sourceRole.created_issue": "The Issue it will create",
  "label.sourceRole.downstream_action_request": "A downstream Action Request",
  "label.sourceRole.downstream_action": "A downstream Action",
  "label.sourceRole.resulting_action_request": "A follow-up Action Request",
  "label.sourceRole.evidence": "Evidence",
  "label.sourceRole.human_judgment": "Human judgment",

  // What is known about an Evidence file's integrity.
  "label.verification.verified": "Verified",
  "label.verification.degraded_last_verified": "Verified before, can't be confirmed now",
  "label.verification.unverified": "Not verified",
  "label.verification.integrity_mismatch": "Doesn't match the record",
  "label.verification.observed_unpinned": "Readable, but no pinned fingerprint",

  // What an Evidence reference is used to show.
  "label.evidenceRole.action_completion": "Shows the Action is complete",
  "label.evidenceRole.decision_resolution": "Supports the Decision",
  "label.evidenceRole.issue_resolution": "Shows the Issue is resolved",
  "label.evidenceRole.issue_closure_verification": "Confirms the Issue can close",
  "label.evidenceRole.issue_failed_verification": "The verification failed",

  // How a change is supported.
  "label.disposition.evidence_satisfied": "Evidence is sufficient",
  "label.disposition.judgment_satisfied": "Covered by human judgment",
  "label.disposition.verification_pending":
    "Evidence is waiting to be verified; proceeding on human judgment",
  "label.disposition.proceed_with_documented_rationale": "Proceed with a written rationale",

  "label.resolutionType.resolved": "Resolved",
  "label.resolutionType.workaround": "Worked around",
  "label.resolutionType.accepted_impact": "Impact accepted",

  // The policy outcome, who may approve, and whether approval can be undone.
  "label.policy.allowed": "Allowed by policy",
  "label.policy.head_of_products": "Approved by the Head of Products",
  "label.policy.not_cancellable_after_submit": "Can't be withdrawn once approved",

  // Classification names, in full English everywhere as the design system
  // requires.
  "label.classification.public": "Public",
  "label.classification.internal": "Internal",
  "label.classification.confidential": "Confidential",
  "label.classification.restricted": "Restricted",
  "label.classification.unclassified": "Unclassified",

  // The module that owns a record -- never the person responsible for it.
  "label.owner.portfolio": "Portfolio management",
  "label.owner.delivery": "Delivery management",
  "label.owner.action_management": "Action management",
  "label.owner.decisions": "Decision management",
  "label.owner.risks": "Risk management",
  "label.owner.issues": "Issue management",
  "label.owner.stakeholders": "Stakeholder management",
  "label.owner.evidence": "Evidence management",
  "label.owner.kpi": "KPI management",
  "label.owner.review": "Review",
} as const;
