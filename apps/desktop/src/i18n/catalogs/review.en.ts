/**
 * English copy for O03, the H2a focused review: what the person is about to
 * approve, where its classification comes from, what supports it, and what
 * happened when they decided.
 */
export const REVIEW_EN = {
  "review.expired": "Expired",
  "review.remaining": "{minutes} min {seconds} s left",
  "review.recordWithVersion": "{id} (version {version})",
  "review.listSeparator": "; ",
  "review.idSeparator": ", ",
  "review.none": "None",
  "review.noLinkedEvidence": "No linked Evidence",
  "review.evidenceBinding": "{id}: {classification}",

  "review.field.actionRequest": "Action Request",
  "review.field.actionToCreate": "Action to create",
  "review.field.subject": "Subject",
  "review.field.commitment": "Commitment",
  "review.field.owner": "Owner",
  "review.field.dueAt": "Due",
  "review.field.actionClassification": "Action classification",
  "review.field.actionToComplete": "Action to complete",
  "review.field.actionToCancel": "Action to cancel",
  "review.field.cancelReason": "Reason for cancelling",
  "review.field.linkedEvidenceClassification": "Classification of linked Evidence",
  "review.field.actionToReopen": "Action to reopen",
  "review.field.mode": "Mode",
  "review.mode.reopenCompleted": "Reopen a completed Action",
  "review.mode.restartCancelled": "Restart a cancelled Action",
  "review.field.reopenReason": "Reason for reopening",
  "review.field.decisionRequest": "Decision Request",
  "review.field.decisionToCreate": "Decision to create",
  "review.field.statement": "Decision",
  "review.field.decisionRationale": "Rationale",
  "review.field.impact": "Impact",
  "review.field.decisionOwner": "Decided by",
  "review.field.decidedAt": "Decided at",
  "review.field.decisionClassification": "Decision classification",
  "review.field.followUps": "Follow-up Action Requests",
  "review.followUp": "{id}: {subject} (owner {owner}, due {due}, {classification})",
  "review.field.riskOccurred": "Risk that occurred",
  "review.field.issueToCreate": "Issue to create",
  "review.field.issueClassification": "Issue classification",
  "review.field.riskToClose": "Risk to close",
  "review.field.closeReason": "Reason for closing",
  "review.field.issueToResolve": "Issue to resolve",
  "review.field.resolutionType": "Resolution",
  "review.field.resolutionRationale": "Rationale",
  "review.field.issueToClose": "Issue to close",
  "review.field.issueToReopen": "Issue to reopen",

  "review.field.targets": "Records to change",
  "review.target": "{kind} {id} (version {version})",
  "review.field.effects": "What approving does",
  "review.effect": "{effect} ({ids})",
  "review.field.classificationSources": "Where the classification comes from",
  "review.classificationSource": "{role}: {classification}",
  "review.classificationSourceWithId": "{role} {id}: {classification}",
  "review.field.policy": "Policy and authority",
  "review.field.support": "Evidence and judgment",
  "review.field.digest": "Digest of what you approve",
  "review.field.preparation": "Preparation record",
  "review.meta.operation": "Operation",
  "review.meta.id": "ID",
  "review.meta.contractVersion": "Contract version",
  "review.meta.preparedAt": "Prepared at",
  "review.meta.correlation": "Correlation ID",
  "review.field.validUntil": "Preview valid until",
  "review.validUntil": "{time} ({remaining})",

  "review.support.none": "This change doesn't rest on Evidence or a Judgment.",
  "review.support.summary": "{disposition} (classification {classification})",
  // One whole sentence per combination, so each language orders the
  // optional clauses itself.
  "review.support.evidence":
    "Evidence {id} ({role}, source version {version}, {classification}): {verification}",
  "review.support.evidenceAt":
    "Evidence {id} ({role}, source version {version}, {classification}): {verification}, at {time}",
  "review.support.evidenceDigest":
    "Evidence {id} ({role}, source version {version}, {classification}): {verification}, digest {digest}",
  "review.support.evidenceAtDigest":
    "Evidence {id} ({role}, source version {version}, {classification}): {verification}, at {time}, digest {digest}",
  "review.support.judgment": "Human judgment ({disposition}, {classification}): {rationale}",

  "review.status.approving": "Approving…",
  "review.status.rejecting": "Recording the rejection…",
  "review.status.approved": "Approved and carried out. {summary}",
  "review.status.rejected": "Rejected.",
  "review.status.rejectedExpired": "Rejected (the preview had already expired).",
  "review.status.expired":
    "This preview has expired and can't be approved. You can still reject it or prepare it again.",
  "review.failed.approve": "The approval didn't finish. {message}",
  "review.failed.reject": "The rejection didn't finish. {message}",
  "review.unknownApproveOutcome":
    "Whether the approval went through is unknown. Retry the same decision.",
  "review.unknownRejectOutcome":
    "Whether the rejection went through is unknown. Retry the same decision.",
  "review.button.reject": "Reject",
  "review.button.later": "Decide later",
  "review.button.prepareAgain": "Prepare again",
  "review.button.close": "Close",
} as const;
