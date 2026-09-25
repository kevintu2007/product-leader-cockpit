/** English copy for S03, the Work Queue, and the forms it opens. */
export const WORK_QUEUE_EN = {
  "workQueue.caption.one":
    "Ordered by the approved ranking policy. Showing {from}–{to} of {count} item.",
  "workQueue.caption.other":
    "Ordered by the approved ranking policy. Showing {from}–{to} of {count} items.",

  // What each row offers.
  "wq.offer.prepareAccept": "Prepare to accept",
  "wq.offer.decline": "Decline",
  "wq.offer.withdraw": "Withdraw",
  "wq.offer.start": "Start",
  "wq.offer.link": "Link Evidence",
  "wq.offer.prepareComplete": "Prepare to complete",
  "wq.offer.prepareCancel": "Prepare to cancel",
  "wq.offer.prepareReopen": "Prepare to reopen",
  "wq.offer.prepareResolve": "Prepare to resolve",
  "wq.offer.prepareOccurrence": "Prepare to record occurrence",
  "wq.offer.prepareClose": "Prepare to close",

  "wq.noDeadline": "No deadline recorded",
  "wq.noResponseDeadline": "No response deadline",

  // The focused review's title, summary and approve button, per operation.
  "wq.review.accept.title": "Approve and carry out: accept {label}",
  "wq.review.accept.summary":
    "Approving creates an Action from the summary you confirmed and links this Request to it. A rejection is recorded, and this preview can't be approved afterwards.",
  "wq.review.accept.approve": "Approve and accept",
  "wq.review.complete.title": "Approve and carry out: complete {label}",
  "wq.review.complete.summary":
    "Approving marks this Action complete, witnessed by the Evidence or Judgment in the preview. A rejection is recorded, and this preview can't be approved afterwards.",
  "wq.review.complete.approve": "Approve and complete",
  "wq.review.cancel.title": "Approve and carry out: cancel {label}",
  "wq.review.cancel.summary":
    "Approving cancels this Action for the reason you wrote. A rejection is recorded, and this preview can't be approved afterwards.",
  "wq.review.cancel.approve": "Approve and cancel",
  "wq.review.reopen.title": "Approve and carry out: reopen {label}",
  "wq.review.reopen.summary":
    "Approving reopens this Action in the mode you chose. A rejection is recorded, and this preview can't be approved afterwards.",
  "wq.review.reopen.approve": "Approve and reopen",
  "wq.review.resolveDecision.title": "Approve and carry out: resolve {label}",
  "wq.review.resolveDecision.summary":
    "Approving creates the Decision in the preview, and each follow-up Action Request it lists. A rejection is recorded, and this preview can't be approved afterwards.",
  "wq.review.resolveDecision.approve": "Approve and resolve",
  "wq.review.occurrence.title": "Approve and carry out: record that {label} occurred",
  "wq.review.occurrence.summary":
    "Approving marks this Risk as occurred and creates the Issue {issue} named in the preview (the app assigned that id; the one you see is the one that will be written). A rejection is recorded, and this preview can't be approved afterwards.",
  "wq.review.occurrence.approve": "Approve and record occurrence",
  "wq.review.closeRisk.title": "Approve and carry out: close {label}",
  "wq.review.closeRisk.summary":
    "Approving closes this Risk for the reason you wrote. A rejection is recorded, and this preview can't be approved afterwards.",
  "wq.review.closeRisk.approve": "Approve and close",
  "wq.review.resolveIssue.title": "Approve and carry out: resolve {label}",
  "wq.review.resolveIssue.summary":
    "Approving marks this Issue resolved with the Evidence in the preview. A rejection is recorded, and this preview can't be approved afterwards.",
  "wq.review.resolveIssue.approve": "Approve and resolve",
  "wq.review.closeIssue.title": "Approve and carry out: close {label}",
  "wq.review.closeIssue.summary":
    "Approving closes this Issue with the verification Evidence in the preview. A rejection is recorded, and this preview can't be approved afterwards.",
  "wq.review.closeIssue.approve": "Approve and close",
  "wq.review.reopenIssue.title": "Approve and carry out: reopen {label}",
  "wq.review.reopenIssue.summary":
    "Approving reopens this Issue with the reason you wrote and the Evidence of the failed verification. A rejection is recorded, and this preview can't be approved afterwards.",
  "wq.review.reopenIssue.approve": "Approve and reopen",

  // What a finished action says.
  "wq.done.declined": "Declined {id} (version {version}).",
  "wq.done.withdrawn": "Withdrew {id} (version {version}).",
  "wq.done.started": "Started {id} (version {version}).",
  "wq.done.linked": "Linked Evidence {evidence} to {id} (version {version}).",
  "wq.noWritePath": "This app has no write path.",
  "wq.settled.completed": "Completed {id} (version {version}, state: {state}).",
  "wq.settled.cancelled": "Cancelled {id} (version {version}, state: {state}).",
  "wq.settled.reopened": "Reopened {id} (version {version}, state: {state}).",
  "wq.settled.accepted": "Created Action {action} and linked it to {request}. Receipt {receipt}.",
  "wq.settled.occurred":
    "Recorded the occurrence (the Risk is now at version {version}) and created Issue {issue}.",
  "wq.settled.riskClosed": "Closed {id} (version {version}).",
  "wq.settled.issue": "{id} is now {state} (version {version}).",
  "wq.settled.decision.one":
    "Created Decision {decision} and {count} follow-up Action Request. Receipt {receipt}.",
  "wq.settled.decision.other":
    "Created Decision {decision} and {count} follow-up Action Requests. Receipt {receipt}.",
  "wq.notice.gone": "{label} is no longer in the list, so it wasn't prepared again.",
  "wq.notice.notAcceptable": "{id} can no longer be accepted, so it wasn't prepared again.",
  "wq.notice.rejected": "The earlier preview was rejected. Enter it again, then prepare it again.",
  "wq.notice.held":
    "Closed the review of “{label}” for now; it was neither accepted nor rejected. To continue, use “Back to the review” on its row; once the preview expires, the review offers to prepare it again.",

  // The detail sheet.
  "wq.detail.close": "Close",
  "wq.detail.attention": "Needs attention",
  "wq.detail.noAttention": "Nothing needs attention",
  "wq.uncertainty": "(the facts behind it are {freshness}; they may no longer hold)",
  "wq.uncertaintyDegraded":
    "(the facts behind it are {freshness} and the source is degraded; they may no longer hold)",
  "wq.detail.deadline": "Deadline",
  "wq.detail.promised": "Promised completion",
  "wq.detail.placement": "Why it's placed here",
  "wq.detail.allowed": "The state allows",
  "wq.none": "None",
  "wq.detail.owner": "Record from",
  "wq.detail.version": "Version",
  "wq.detail.next": "Next step",

  // Row actions and forms.
  "wq.backToReview": "Back to the review",
  "wq.sending": "Sending…",
  "wq.actionFailed": "This action didn't finish. {message}",
  "wq.abandon": "Give up this action",
  "wq.cancel": "Cancel",
  "wq.reason.closeRisk": "Reason for closing",
  "wq.reason.decline": "Reason for declining",
  "wq.reason.withdraw": "Reason for withdrawing",
  "wq.reason.cancel": "Reason for cancelling",
  "wq.reason.reopen": "Reason for reopening",
  "wq.confirm.closePreview": "Prepare the closing preview",
  "wq.confirm.decline": "Confirm decline",
  "wq.confirm.withdraw": "Confirm withdrawal",
  "wq.confirm.cancelPreview": "Prepare the cancellation preview",
  "wq.confirm.reopenPreview": "Prepare the reopening preview",
  "wq.confirm.resolvePreview": "Prepare the resolution preview",
  "wq.confirm.completePreview": "Prepare the completion preview",
  "wq.reopenMode": "Reopen mode",
  "wq.reopenMode.completed": "Reopen a completed Action",
  "wq.reopenMode.cancelled": "Restart a cancelled Action",
  "wq.issueEvidence.resolve":
    "Evidence that this Issue is resolved (choose any; all three transitions need Evidence)",
  "wq.issueEvidence.close": "Evidence that the resolution was verified (choose any)",
  "wq.issueEvidence.reopen": "Evidence that the verification failed (choose any)",
  "wq.resolutionType": "Resolution",
  "wq.resolutionType.resolved": "Resolved",
  "wq.resolutionType.workaround": "Worked around",
  "wq.resolutionType.acceptedImpact": "Impact accepted",
  "wq.reason.resolve": "Reason for resolving",
  "wq.evidenceLoading": "Reading Evidence…",
  "wq.evidenceNone": "There's no Evidence in the Ledger.",
  "wq.evidenceOption.pinned": "{id}: {verification}, pinned, {classification} (version {version})",
  "wq.evidenceOption.unpinned":
    "{id}: {verification}, not pinned, {classification} (version {version})",
  "wq.judgment.issue":
    "Judgment rationale (leave empty to attach none; it's required if the chosen Evidence is only partly verified, and it can't carry Evidence that is unverified or doesn't match its record)",
  "wq.judgment.complete":
    "Judgment rationale (leave empty to attach none; it's required if the linked Evidence isn't verified)",
  "wq.judgment.decision": "Judgment rationale (leave empty to attach none)",
  "wq.judgmentClassification": "Judgment classification",
  "wq.linkLoading": "Reading the Evidence you can link…",
  "wq.linkChoose": "Evidence to link",
  "wq.linkPlaceholder": "(choose one)",
  "wq.linkNone": "There's no Evidence in the Ledger left to link.",
  "wq.linkConfirm": "Confirm link",
  "wq.decision.statement": "Decision",
  "wq.decision.rationale": "Rationale",
  "wq.decision.impact": "Impact",
  "wq.decision.evidence":
    "Evidence that supports this Decision (choose any; without Evidence a Judgment is required)",
  "wq.followUps": "Follow-up Action Requests (the app assigns each one an id)",
  "wq.followUp.subject": "Subject",
  "wq.followUp.details": "Details",
  "wq.followUp.owner": "Owner (Stakeholder id)",
  "wq.followUp.due": "Due",
  "wq.followUp.classification": "Classification",
  "wq.followUp.remove": "Remove this one",
  "wq.followUp.add": "Add a follow-up Action Request",

  // The page.
  "wq.outOfSync":
    "The two sources of the Work Queue read different Ledger revisions, so no items are shown. Joining two moments into one list would look right and be wrong.",
  "wq.headline": "What to handle today",
  "wq.lede":
    "In order: commitments missed, work blocked, Evidence with problems, deadlines near, nobody accountable. Each item says why it's placed where it is.",
  "wq.filters": "Filter by kind (with none ticked, everything shows)",
  "wq.filterChip": "{kind} ({count})",
  "wq.onlyFlagged": "Only items that need attention",
  "wq.empty": "Nothing to do right now.",
  "wq.column.kind": "Kind",
  "wq.column.item": "Item",
  "wq.column.attention": "Needs attention",
  "wq.column.deadline": "Deadline",
  "wq.column.placement": "Why it's placed here",
  "wq.column.next": "Next step",
  "wq.allowed": "The state allows: {intents}",
  "wq.promised": "Promised completion {date}",
  "wq.inDetail": "Act in the detail sheet",
  "wq.previous": "Previous page",
  "wq.next": "Next page",
} as const;
