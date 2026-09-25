import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";

import type {
  AcceptedActionDto,
  ActionCompletionContextDto,
  ActionOutcomeDto,
  DecisionRequestOutcomeDto,
  EvidenceReferencesDto,
  ActionRequestOutcomeDto,
  IssueOutcomeDto,
  OccurredRiskDto,
  PreparedIntentDto,
  RejectedPreparedIntentDto,
  ResolvedDecisionDto,
  RiskOutcomeDto,
  SafeErrorDto,
  WorkQueueDto,
  WorkQueueItemDto,
} from "./cockpitContract";
import { type HeldReviews, WorkQueue, type WorkQueueActions } from "./WorkQueue";

const NOW = 1_700_000_000_000;
const DIGEST = "e".repeat(64);

function openRequest(): WorkQueueItemDto {
  return {
    kind: "action_request",
    id: "request-1",
    label: "Action Request request-1",
    stateLabel: "Open",
    classification: "internal",
    revision: 2,
    owner: "action-management",
    asOfMillis: NOW,
    lifecycleLegalIntents: [
      "prepare_accept_action_request",
      "decline_action_request",
      "withdraw_action_request",
    ],
    relevantAtMillis: NOW + 1_000,
    placement: "unflagged",
    placementRationale: "nothing has flagged it",
    attention: [],
  };
}

function openAction(): WorkQueueItemDto {
  return {
    kind: "action",
    id: "action-1",
    label: "Action action-1",
    stateLabel: "Open",
    classification: "internal",
    revision: 1,
    owner: "action-management",
    asOfMillis: NOW,
    lifecycleLegalIntents: ["start_action", "prepare_cancel_action"],
    relevantAtMillis: NOW + 1_000,
    placement: "unflagged",
    placementRationale: "nothing has flagged it",
    attention: [],
  };
}

function queue(items: readonly WorkQueueItemDto[]): WorkQueueDto {
  return {
    state: "success",
    asOfMillis: NOW,
    ledgerRevision: 7,
    items,
    countsByKind: [
      { kind: "action_request", count: items.filter((i) => i.kind === "action_request").length },
      { kind: "action", count: items.filter((i) => i.kind === "action").length },
      { kind: "decision_request", count: 0 },
      { kind: "risk", count: 0 },
      { kind: "issue", count: 0 },
    ],
    offset: 0,
    limit: 25,
    total: items.length,
    hasMore: false,
  };
}

function preparedDto(): PreparedIntentDto {
  return {
    preparedIntentId: "prepared-1",
    contractVersion: 1,
    intentType: "accept_action_request",
    operation: {
      kind: "acceptActionRequest",
      requestId: "request-1",
      requestVersion: 2,
      actionId: "action-9",
      actionClassification: "internal",
      actionSubject: "Synthetic subject",
      commitmentDetails: "Synthetic commitment",
      intendedOwner: "owner-1",
      intendedDueAtMillis: NOW + 86_400_000,
    },
    targets: [{ kind: "action_request", id: "request-1", expectedVersion: 2 }],
    declaredEffects: [{ kind: "accept_action_request", ids: ["request-1"] }],
    payloadDigest: DIGEST,
    resolvedClassification: "internal",
    classificationSources: [{ role: "primary_target", id: null, classification: "internal" }],
    support: null,
    policyResult: "allowed",
    authority: "head_of_products",
    preparedAtMillis: NOW,
    expiresAtMillis: NOW + 300_000,
    cancellationPolicy: "not_cancellable_after_submit",
    correlationId: "host-1",
  };
}

function accepted(): AcceptedActionDto {
  return {
    request: {
      id: "request-1",
      title: "Synthetic subject",
      details: "Synthetic commitment",
      intendedOwner: "owner-1",
      responseDueAtMillis: null,
      intendedActionDueAtMillis: NOW + 86_400_000,
      classification: "internal",
      state: "accepted",
      version: 3,
      terminalRationale: null,
      linkedActionId: "action-9",
    },
    action: {
      id: "action-9",
      sourceRequestId: "request-1",
      title: "Synthetic subject",
      details: "Synthetic commitment",
      owner: "owner-1",
      dueAtMillis: NOW + 86_400_000,
      classification: "internal",
      state: "open",
      version: 1,
    },
    auditEventIds: ["a1", "a2", "a3"],
    approvalReceiptId: "receipt-1",
    correlationId: "host-2",
  };
}

function safeError(errorCode: string, messageKey: string, retryable: boolean): SafeErrorDto {
  return {
    errorCode,
    messageKey,
    messageParams: [],
    correlationId: "host-err",
    retryable,
    extensions: [],
  };
}

function actionsMock(overrides: Partial<WorkQueueActions> = {}): WorkQueueActions {
  return {
    prepareAcceptActionRequest: vi.fn(() => Promise.resolve(preparedDto())),
    approveAndExecuteAcceptActionRequest: vi.fn(() => Promise.resolve(accepted())),
    rejectPreparedAcceptActionRequest: vi.fn(() =>
      Promise.resolve<RejectedPreparedIntentDto>({
        preparedIntentId: "prepared-1",
        disposition: "rejected",
        rejectedAtMillis: NOW + 5_000,
        expiredAtRejection: false,
        correlationId: "host-3",
      }),
    ),
    declineActionRequest: vi.fn(() =>
      Promise.resolve<ActionRequestOutcomeDto>({
        request: { ...accepted().request, state: "declined", version: 3, linkedActionId: null },
        auditEventIds: ["a4"],
        correlationId: "host-4",
      }),
    ),
    withdrawActionRequest: vi.fn(() =>
      Promise.resolve<ActionRequestOutcomeDto>({
        request: { ...accepted().request, state: "withdrawn", version: 3, linkedActionId: null },
        auditEventIds: ["a5"],
        correlationId: "host-5",
      }),
    ),
    startAction: vi.fn(() =>
      Promise.resolve<ActionOutcomeDto>({
        action: { ...accepted().action, id: "action-1", state: "in_progress", version: 2 },
        auditEventIds: ["a6"],
        correlationId: "host-6",
      }),
    ),
    loadActionCompletionContext: vi.fn(() =>
      Promise.resolve<ActionCompletionContextDto>({
        action: { ...accepted().action, id: "action-1", state: "in_progress", version: 2 },
        linkedEvidenceIds: [],
        ledgerRevision: 7,
        evidenceReferences: [
          {
            id: "evidence-1",
            role: null,
            verification: { kind: "verified", atMillis: NOW, integrityDigest: "a".repeat(64) },
            pinned: true,
            classification: "internal",
            version: 1,
          },
        ],
        correlationId: "host-7",
      }),
    ),
    linkActionCompletionEvidence: vi.fn(() =>
      Promise.resolve<ActionOutcomeDto>({
        action: { ...accepted().action, id: "action-1", state: "in_progress", version: 3 },
        auditEventIds: ["a7"],
        correlationId: "host-8",
      }),
    ),
    prepareCompleteAction: vi.fn(() =>
      Promise.resolve<PreparedIntentDto>({
        ...preparedDto(),
        preparedIntentId: "prepared-complete-1",
        intentType: "complete_action",
        operation: { kind: "completeAction", actionId: "action-1", actionVersion: 3 },
      }),
    ),
    prepareCancelAction: vi.fn(() =>
      Promise.resolve<PreparedIntentDto>({
        ...preparedDto(),
        preparedIntentId: "prepared-cancel-1",
        intentType: "cancel_action",
        operation: {
          kind: "cancelAction",
          actionId: "action-1",
          actionVersion: 3,
          reason: "Scope moved.",
          evidenceClassifications: [],
        },
      }),
    ),
    prepareReopenAction: vi.fn(() =>
      Promise.resolve<PreparedIntentDto>({
        ...preparedDto(),
        preparedIntentId: "prepared-reopen-1",
        intentType: "reopen_action",
        operation: {
          kind: "reopenAction",
          actionId: "action-1",
          actionVersion: 4,
          mode: "reopen_completed",
          reason: "It came back.",
          evidenceClassifications: [],
        },
      }),
    ),
    approveAndExecuteCompleteAction: vi.fn(() =>
      Promise.resolve<ActionOutcomeDto>({
        action: { ...accepted().action, id: "action-1", state: "completed", version: 4 },
        auditEventIds: ["a8"],
        correlationId: "host-9",
      }),
    ),
    approveAndExecuteCancelAction: vi.fn(() =>
      Promise.resolve<ActionOutcomeDto>({
        action: { ...accepted().action, id: "action-1", state: "cancelled", version: 4 },
        auditEventIds: ["a9"],
        correlationId: "host-10",
      }),
    ),
    approveAndExecuteReopenAction: vi.fn(() =>
      Promise.resolve<ActionOutcomeDto>({
        action: { ...accepted().action, id: "action-1", state: "open", version: 5 },
        auditEventIds: ["a10"],
        correlationId: "host-11",
      }),
    ),
    rejectPreparedActionIntent: vi.fn(() =>
      Promise.resolve<RejectedPreparedIntentDto>({
        preparedIntentId: "prepared-cancel-1",
        disposition: "rejected",
        rejectedAtMillis: NOW + 5_000,
        expiredAtRejection: false,
        correlationId: "host-12",
      }),
    ),
    loadEvidenceReferences: vi.fn(() =>
      Promise.resolve<EvidenceReferencesDto>({
        ledgerRevision: 7,
        evidenceReferences: [
          {
            id: "evidence-1",
            role: null,
            verification: { kind: "verified", atMillis: NOW, integrityDigest: "a".repeat(64) },
            pinned: true,
            classification: "internal",
            version: 1,
          },
        ],
        correlationId: "host-13",
      }),
    ),
    withdrawDecisionRequest: vi.fn(() =>
      Promise.resolve<DecisionRequestOutcomeDto>({
        request: {
          id: "decision-request-1",
          subject: "Choose direction",
          details: "Synthetic",
          intendedOwner: "owner-1",
          classification: "internal",
          state: "withdrawn",
          version: 3,
          withdrawalRationale: "No longer needed.",
          linkedDecisionId: null,
        },
        auditEventIds: ["d1"],
        correlationId: "host-14",
      }),
    ),
    prepareResolveDecisionRequest: vi.fn(() =>
      Promise.resolve<PreparedIntentDto>({
        ...preparedDto(),
        preparedIntentId: "prepared-resolve-1",
        intentType: "resolve_decision_request",
        operation: {
          kind: "resolveDecisionRequest",
          requestId: "decision-request-1",
          requestVersion: 2,
          decisionId: "decision-9",
          decisionClassification: "internal",
          statement: "Proceed with option A.",
          rationale: "Best tradeoff.",
          impact: "On track.",
          decisionOwner: "owner-1",
          decidedAtMillis: NOW,
          resultingActionRequests: [
            {
              id: "request-9",
              subject: "Follow up",
              details: "Do the follow-up.",
              intendedOwner: "owner-1",
              dueAtMillis: NOW + 86_400_000,
              classification: "internal",
            },
          ],
        },
      }),
    ),
    approveAndExecuteResolveDecisionRequest: vi.fn(() =>
      Promise.resolve<ResolvedDecisionDto>({
        request: {
          id: "decision-request-1",
          subject: "Choose direction",
          details: "Synthetic",
          intendedOwner: "owner-1",
          classification: "internal",
          state: "resolved",
          version: 3,
          withdrawalRationale: null,
          linkedDecisionId: "decision-9",
        },
        decision: {
          id: "decision-9",
          sourceRequestId: "decision-request-1",
          statement: "Proceed with option A.",
          rationale: "Best tradeoff.",
          impact: "On track.",
          owner: "owner-1",
          decidedAtMillis: NOW,
          classification: "internal",
          state: "effective",
          version: 1,
          resultingActionRequestIds: ["request-9"],
        },
        resultingActionRequestIds: ["request-9"],
        auditEventIds: ["d2", "d3", "d4"],
        approvalReceiptId: "receipt-9",
        correlationId: "host-15",
      }),
    ),
    rejectPreparedDecisionIntent: vi.fn(() =>
      Promise.resolve<RejectedPreparedIntentDto>({
        preparedIntentId: "prepared-resolve-1",
        disposition: "rejected",
        rejectedAtMillis: NOW + 5_000,
        expiredAtRejection: false,
        correlationId: "host-16",
      }),
    ),
    // Risk and Issue.
    prepareRecordRiskOccurrence: vi.fn(() => Promise.resolve(occurrencePreparedDto())),
    prepareCloseRisk: vi.fn(() => Promise.resolve(closeRiskPreparedDto())),
    approveAndExecuteRecordRiskOccurrence: vi.fn(() =>
      Promise.resolve<OccurredRiskDto>({
        risk: {
          id: "risk-1",
          title: "Vendor concentration",
          details: "Synthetic.",
          classification: "internal",
          state: "occurred",
          version: 2,
        },
        issue: {
          id: "issue-from-occurrence",
          title: "Vendor concentration",
          details: "Synthetic.",
          classification: "internal",
          state: "open",
          version: 1,
          sourceRiskId: "risk-1",
          resolutionType: null,
        },
        auditEventIds: ["r1", "r2", "r3"],
        approvalReceiptId: "receipt-risk-1",
        correlationId: "host-17",
      }),
    ),
    approveAndExecuteCloseRisk: vi.fn(() =>
      Promise.resolve<RiskOutcomeDto>({
        risk: {
          id: "risk-1",
          title: "Vendor concentration",
          details: "Synthetic.",
          classification: "internal",
          state: "closed",
          version: 2,
        },
        auditEventIds: ["r4"],
        approvalReceiptId: "receipt-risk-2",
        correlationId: "host-18",
      }),
    ),
    rejectPreparedRiskIntent: vi.fn(() =>
      Promise.resolve<RejectedPreparedIntentDto>({
        preparedIntentId: "prepared-occurrence-1",
        disposition: "rejected",
        rejectedAtMillis: NOW + 5_000,
        expiredAtRejection: false,
        correlationId: "host-19",
      }),
    ),
    prepareResolveIssue: vi.fn(() => Promise.resolve(resolveIssuePreparedDto())),
    prepareCloseIssue: vi.fn(() => Promise.resolve(resolveIssuePreparedDto())),
    prepareReopenIssue: vi.fn(() => Promise.resolve(resolveIssuePreparedDto())),
    approveAndExecuteIssueTransition: vi.fn(() =>
      Promise.resolve<IssueOutcomeDto>({
        issue: {
          id: "issue-1",
          title: "Nightly export fails",
          details: "Synthetic.",
          classification: "internal",
          state: "resolved",
          version: 2,
          sourceRiskId: null,
          resolutionType: "resolved",
        },
        auditEventIds: ["i1"],
        approvalReceiptId: "receipt-issue-1",
        correlationId: "host-20",
      }),
    ),
    rejectPreparedIssueIntent: vi.fn(() =>
      Promise.resolve<RejectedPreparedIntentDto>({
        preparedIntentId: "prepared-resolve-issue-1",
        disposition: "rejected",
        rejectedAtMillis: NOW + 5_000,
        expiredAtRejection: false,
        correlationId: "host-21",
      }),
    ),
    ...overrides,
  };
}

function occurrencePreparedDto(): PreparedIntentDto {
  return {
    ...preparedDto(),
    preparedIntentId: "prepared-occurrence-1",
    intentType: "record_risk_occurrence",
    operation: {
      kind: "recordRiskOccurrence",
      riskId: "risk-1",
      riskVersion: 1,
      issueId: "issue-from-occurrence",
      issueClassification: "internal",
    },
  };
}

function closeRiskPreparedDto(): PreparedIntentDto {
  return {
    ...preparedDto(),
    preparedIntentId: "prepared-close-risk-1",
    intentType: "close_risk",
    operation: {
      kind: "closeRisk",
      riskId: "risk-1",
      riskVersion: 1,
      rationale: "The vendor was replaced.",
    },
  };
}

function resolveIssuePreparedDto(): PreparedIntentDto {
  return {
    ...preparedDto(),
    preparedIntentId: "prepared-resolve-issue-1",
    intentType: "resolve_issue",
    operation: {
      kind: "resolveIssue",
      issueId: "issue-1",
      issueVersion: 1,
      resolutionType: "resolved",
      rationale: "The export was fixed.",
    },
  };
}

function openRisk(): WorkQueueItemDto {
  return {
    ...openRequest(),
    kind: "risk",
    id: "risk-1",
    label: "Risk risk-1",
    stateLabel: "Open",
    lifecycleLegalIntents: ["prepare_record_risk_occurrence", "prepare_close_risk"],
    revision: 1,
  };
}

function openIssue(): WorkQueueItemDto {
  return {
    ...openRequest(),
    kind: "issue",
    id: "issue-1",
    label: "Nightly export fails",
    stateLabel: "Open",
    lifecycleLegalIntents: ["prepare_resolve_issue"],
    revision: 1,
  };
}

function resolvedIssue(): WorkQueueItemDto {
  return {
    ...openIssue(),
    stateLabel: "Resolved",
    lifecycleLegalIntents: ["prepare_close_issue", "prepare_reopen_issue"],
    revision: 2,
  };
}

function openDecisionRequest(): WorkQueueItemDto {
  return {
    kind: "decision_request",
    id: "decision-request-1",
    label: "Decision Request decision-request-1",
    stateLabel: "Open",
    classification: "internal",
    revision: 2,
    owner: "decision-management",
    asOfMillis: NOW,
    lifecycleLegalIntents: ["withdraw_decision_request", "prepare_resolve_decision_request"],
    relevantAtMillis: null,
    placement: "unflagged",
    placementRationale: "nothing has flagged it",
    attention: [],
  };
}

function inProgressAction(): WorkQueueItemDto {
  return {
    ...openAction(),
    stateLabel: "InProgress",
    revision: 2,
    lifecycleLegalIntents: [
      "link_action_completion_evidence",
      "prepare_complete_action",
      "prepare_cancel_action",
    ],
  };
}

function completedAction(): WorkQueueItemDto {
  return {
    ...openAction(),
    stateLabel: "Completed",
    revision: 4,
    lifecycleLegalIntents: ["prepare_reopen_action"],
  };
}

function ids() {
  let counter = 0;
  return () => {
    counter += 1;
    return `req-${String(counter)}`;
  };
}

describe("WorkQueue actions", () => {
  it("offers no action at all when the host has no write path", async () => {
    render(<WorkQueue load={() => Promise.resolve(queue([openRequest(), openAction()]))} />);
    const row = await screen.findByRole("row", { name: /Action Request request-1/ });
    // The one control is the title, which only opens a read-only detail.
    expect(within(row).getAllByRole("button")).toHaveLength(1);
    expect(within(row).getByRole("button")).toHaveClass("pmc-work-item-open");
    expect(screen.queryByRole("columnheader", { name: "可進入的受管路徑" })).toBeNull();
  });

  it("offers only the governed paths the item's lifecycle admits, next to the intents text", async () => {
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([openRequest(), openAction()]))}
        actions={actionsMock()}
        newClientRequestId={ids()}
        now={() => NOW}
      />,
    );
    const request = await screen.findByRole("row", { name: /Action Request request-1/ });
    expect(within(request).getByText(/接受請求/)).toBeInTheDocument();
    expect(within(request).getByRole("button", { name: "準備接受" })).toBeEnabled();
    expect(within(request).getByRole("button", { name: "婉拒" })).toBeEnabled();
    expect(within(request).getByRole("button", { name: "撤回" })).toBeEnabled();
    const action = screen.getByRole("row", { name: /Action action-1/ });
    expect(within(action).getByRole("button", { name: "開始" })).toBeEnabled();
    expect(within(action).getByRole("button", { name: "準備取消" })).toBeEnabled();
    // Only what this Open Action's lifecycle admits, beside the title that
    // opens its detail: no control for Complete or Reopen, which it does not
    // admit yet.
    expect(within(action).getAllByRole("button")).toHaveLength(3);
    expect(within(action).queryByRole("button", { name: "準備完成" })).toBeNull();
  });

  it("prepares the exact preview, then approves with the acknowledged digest and refreshes", async () => {
    const user = userEvent.setup();
    const load = vi.fn(() => Promise.resolve(queue([openRequest()])));
    const actions = actionsMock();
    render(<WorkQueue load={load} actions={actions} newClientRequestId={ids()} now={() => NOW} />);

    await user.click(await screen.findByRole("button", { name: "準備接受" }));

    expect(actions.prepareAcceptActionRequest).toHaveBeenCalledWith("request-1", 2, "req-1");
    expect(await screen.findByRole("dialog")).toBeVisible();
    expect(screen.getByText(DIGEST)).toBeVisible();

    await user.click(screen.getByRole("button", { name: "核准並接受" }));

    await waitFor(() => {
      expect(actions.approveAndExecuteAcceptActionRequest).toHaveBeenCalledWith(
        "prepared-1",
        DIGEST,
        "req-2",
      );
    });
    expect(await screen.findByText(/已建立 Action action-9/)).toBeVisible();
    const loadsBefore = load.mock.calls.length;
    await user.click(screen.getByRole("button", { name: "關閉" }));
    await waitFor(() => {
      expect(load.mock.calls.length).toBeGreaterThan(loadsBefore);
    });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("records a rejection durably with its own request id", async () => {
    const user = userEvent.setup();
    const actions = actionsMock();
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([openRequest()]))}
        actions={actions}
        newClientRequestId={ids()}
        now={() => NOW}
      />,
    );
    await user.click(await screen.findByRole("button", { name: "準備接受" }));
    await screen.findByRole("dialog");

    await user.click(screen.getByRole("button", { name: "拒絕" }));

    await waitFor(() => {
      expect(actions.rejectPreparedAcceptActionRequest).toHaveBeenCalledWith("prepared-1", "req-3");
    });
    expect(actions.approveAndExecuteAcceptActionRequest).not.toHaveBeenCalled();
    expect(await screen.findByText("已拒絕。")).toBeVisible();
  });

  it("closes a preview without deciding: nothing is sent and the row stays as it was", async () => {
    const user = userEvent.setup();
    const actions = actionsMock();
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([openRequest()]))}
        actions={actions}
        newClientRequestId={ids()}
        now={() => NOW}
      />,
    );
    await user.click(await screen.findByRole("button", { name: "準備接受" }));
    await screen.findByRole("dialog");

    await user.click(screen.getByRole("button", { name: "先不決定" }));

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(await screen.findByText(/沒有接受也沒有拒絕/)).toBeVisible();
    expect(actions.rejectPreparedAcceptActionRequest).not.toHaveBeenCalled();
    expect(actions.approveAndExecuteAcceptActionRequest).not.toHaveBeenCalled();

    // The first preview is still pending on the host, so the row must not
    // prepare a second approvable one beside it: it leads back instead.
    expect(screen.getByRole("button", { name: "準備接受" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "回到審閱單" }));
    expect(await screen.findByRole("dialog")).toBeVisible();
    expect(actions.prepareAcceptActionRequest).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("button", { name: "回到審閱單" })).not.toBeInTheDocument();
  });

  it("closes a review with Escape without deciding, like 先不決定", async () => {
    const user = userEvent.setup();
    const actions = actionsMock();
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([openRequest()]))}
        actions={actions}
        newClientRequestId={ids()}
        now={() => NOW}
      />,
    );
    await user.click(await screen.findByRole("button", { name: "準備接受" }));
    await screen.findByRole("dialog");

    await user.keyboard("{Escape}");

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(actions.rejectPreparedAcceptActionRequest).not.toHaveBeenCalled();
    expect(actions.approveAndExecuteAcceptActionRequest).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "回到審閱單" })).toBeEnabled();
  });

  it("opens an item's detail from its title without asking the host for anything", async () => {
    const user = userEvent.setup();
    const load = vi.fn(() => Promise.resolve(queue([openRequest()])));
    const actions = actionsMock();
    render(<WorkQueue load={load} actions={actions} newClientRequestId={ids()} now={() => NOW} />);
    await user.click(await screen.findByRole("button", { name: "Action Request request-1" }));

    const detail = await screen.findByRole("dialog", { name: "Action Request request-1" });
    expect(within(detail).getByText("紀錄來源")).toBeVisible();
    // Labelled facts, not one dot-joined string: the label and its value are
    // two elements, so each can be read on its own.
    expect(within(detail).getByText("版本").nextElementSibling).toHaveTextContent("2");
    // Focus starts on 關閉, so nothing on opening can start a command.
    expect(within(detail).getByRole("button", { name: "關閉" })).toHaveFocus();
    expect(load).toHaveBeenCalledTimes(1);
    expect(actions.prepareAcceptActionRequest).not.toHaveBeenCalled();
    // Its commands live in the sheet alone while it is open, never twice.
    expect(screen.getAllByRole("button", { name: "準備接受" })).toHaveLength(1);
    expect(screen.getByText("在詳情視窗中操作")).toBeInTheDocument();

    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    // Its next steps are the row's own; preparing replaces the detail sheet
    // with the review rather than stacking the two.
    await user.click(screen.getByRole("button", { name: "Action Request request-1" }));
    const again = await screen.findByRole("dialog", { name: "Action Request request-1" });
    await user.click(within(again).getByRole("button", { name: "準備接受" }));
    expect(await screen.findByRole("dialog", { name: /核准並執行/ })).toBeVisible();
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
  });

  it("retries a failed approval with the same request id, never a fresh one", async () => {
    const user = userEvent.setup();
    const actions = actionsMock({
      approveAndExecuteAcceptActionRequest: vi
        .fn<WorkQueueActions["approveAndExecuteAcceptActionRequest"]>()
        .mockRejectedValueOnce(safeError("PLATFORM_INTERNAL", "ledger.transaction.busy", true))
        .mockResolvedValueOnce(accepted()),
    });
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([openRequest()]))}
        actions={actions}
        newClientRequestId={ids()}
        now={() => NOW}
      />,
    );
    await user.click(await screen.findByRole("button", { name: "準備接受" }));
    await screen.findByRole("dialog");
    await user.click(screen.getByRole("button", { name: "核准並接受" }));
    expect(await screen.findByText("host-err")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "重試" }));

    await waitFor(() => {
      expect(actions.approveAndExecuteAcceptActionRequest).toHaveBeenCalledTimes(2);
    });
    const calls = vi.mocked(actions.approveAndExecuteAcceptActionRequest).mock.calls;
    expect(calls[0]?.[2]).toBe("req-2");
    expect(calls[1]?.[2]).toBe("req-2");
  });

  it("declines with the person's own rationale and shows the authoritative outcome", async () => {
    const user = userEvent.setup();
    const actions = actionsMock();
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([openRequest()]))}
        actions={actions}
        newClientRequestId={ids()}
        now={() => NOW}
      />,
    );
    await user.click(await screen.findByRole("button", { name: "婉拒" }));
    expect(screen.getByRole("button", { name: "確認婉拒" })).toBeDisabled();
    await user.type(screen.getByLabelText("婉拒理由"), "Capacity is unavailable");
    await user.click(screen.getByRole("button", { name: "確認婉拒" }));

    await waitFor(() => {
      expect(actions.declineActionRequest).toHaveBeenCalledWith(
        "request-1",
        2,
        "Capacity is unavailable",
        "req-1",
      );
    });
    expect(await screen.findByText("已婉拒 request-1（版本 3）。")).toBeVisible();
  });

  it("starts an Open Action and reports the new version", async () => {
    const user = userEvent.setup();
    const actions = actionsMock();
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([openAction()]))}
        actions={actions}
        newClientRequestId={ids()}
        now={() => NOW}
      />,
    );
    await user.click(await screen.findByRole("button", { name: "開始" }));
    await waitFor(() => {
      expect(actions.startAction).toHaveBeenCalledWith("action-1", 1, "req-1");
    });
    expect(await screen.findByText("已開始 action-1（版本 2）。")).toBeVisible();
  });

  it("renders a refused H1 command through O05 and lets the person abandon it", async () => {
    const user = userEvent.setup();
    const actions = actionsMock({
      startAction: vi.fn(() =>
        // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- the host rejects with its safe envelope, not an Error.
        Promise.reject(safeError("DOMAIN_CONFLICT", "action.domain_conflict", false)),
      ),
    });
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([openAction()]))}
        actions={actions}
        newClientRequestId={ids()}
        now={() => NOW}
      />,
    );
    await user.click(await screen.findByRole("button", { name: "開始" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("這個動作沒有完成。");
    expect(alert).toHaveTextContent("重試不會改變結果。");
    expect(within(alert).queryByRole("button", { name: "重試" })).toBeNull();
    await user.click(within(alert).getByRole("button", { name: "放棄這個動作" }));
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "開始" })).toBeEnabled();
  });

  it("prepare again rejects the pending preview durably, re-reads, and prepares afresh", async () => {
    const user = userEvent.setup();
    const load = vi.fn(() => Promise.resolve(queue([openRequest()])));
    const actions = actionsMock();
    // The clock sits past expiry, so O03 offers "prepare again" at once.
    render(
      <WorkQueue
        load={load}
        actions={actions}
        newClientRequestId={ids()}
        now={() => NOW + 300_001}
      />,
    );
    await user.click(await screen.findByRole("button", { name: "準備接受" }));
    await screen.findByRole("dialog");

    await user.click(screen.getByRole("button", { name: "重新準備" }));

    await waitFor(() => {
      expect(actions.rejectPreparedAcceptActionRequest).toHaveBeenCalledWith("prepared-1", "req-3");
    });
    await waitFor(() => {
      expect(actions.prepareAcceptActionRequest).toHaveBeenCalledTimes(2);
    });
    const calls = vi.mocked(actions.prepareAcceptActionRequest).mock.calls;
    expect(calls[0]?.[2]).toBe("req-1");
    expect(calls[1]?.[2]).toBe("req-4");
    expect(await screen.findByRole("dialog")).toBeVisible();
  });

  it("prepares again even when rejecting the expired preview fails, since it can no longer execute", async () => {
    const user = userEvent.setup();
    const actions = actionsMock({
      rejectPreparedAcceptActionRequest: vi
        .fn<WorkQueueActions["rejectPreparedAcceptActionRequest"]>()
        .mockRejectedValueOnce(safeError("PLATFORM_INTERNAL", "ledger.transaction.busy", true)),
    });
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([openRequest()]))}
        actions={actions}
        newClientRequestId={ids()}
        now={() => NOW + 300_001}
      />,
    );
    await user.click(await screen.findByRole("button", { name: "準備接受" }));
    await screen.findByRole("dialog");

    await user.click(screen.getByRole("button", { name: "重新準備" }));

    await waitFor(() => {
      expect(actions.prepareAcceptActionRequest).toHaveBeenCalledTimes(2);
    });
    expect(actions.rejectPreparedAcceptActionRequest).toHaveBeenCalledTimes(1);
    expect(await screen.findByRole("dialog")).toBeVisible();
  });

  it("keeps a review closed without deciding across leaving and returning to the route", async () => {
    const user = userEvent.setup();
    const actions = actionsMock();
    function Host() {
      const [held, setHeld] = useState<HeldReviews>(() => new Map());
      const [newId] = useState(() => ids());
      const [shown, setShown] = useState(true);
      return (
        <>
          <button
            type="button"
            onClick={() => {
              setShown((value) => !value);
            }}
          >
            切換頁面
          </button>
          {shown && (
            <WorkQueue
              load={() => Promise.resolve(queue([openRequest()]))}
              actions={actions}
              newClientRequestId={newId}
              now={() => NOW}
              heldReviews={held}
              onHeldReviewsChange={setHeld}
            />
          )}
        </>
      );
    }
    render(<Host />);
    await user.click(await screen.findByRole("button", { name: "準備接受" }));
    await screen.findByRole("dialog");
    await user.click(screen.getByRole("button", { name: "先不決定" }));

    await user.click(screen.getByRole("button", { name: "切換頁面" }));
    expect(screen.queryByRole("button", { name: "準備接受" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "切換頁面" }));

    expect(await screen.findByRole("button", { name: "回到審閱單" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "準備接受" })).toBeDisabled();
    expect(actions.prepareAcceptActionRequest).toHaveBeenCalledTimes(1);
  });

  it("retries a refused decline with the same request id and lets the other row wait", async () => {
    const user = userEvent.setup();
    const actions = actionsMock({
      declineActionRequest: vi
        .fn<WorkQueueActions["declineActionRequest"]>()
        .mockRejectedValueOnce(safeError("PLATFORM_INTERNAL", "ledger.transaction.busy", true))
        .mockResolvedValueOnce({
          request: { ...accepted().request, state: "declined", version: 3, linkedActionId: null },
          auditEventIds: ["a4"],
          correlationId: "host-4",
        }),
    });
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([openRequest(), openAction()]))}
        actions={actions}
        newClientRequestId={ids()}
        now={() => NOW}
      />,
    );
    await user.click(await screen.findByRole("button", { name: "婉拒" }));
    // While this row composes, the other row's action waits.
    expect(screen.getByRole("button", { name: "開始" })).toBeDisabled();
    await user.type(screen.getByLabelText("婉拒理由"), "No capacity");
    await user.click(screen.getByRole("button", { name: "確認婉拒" }));
    await screen.findByRole("alert");

    await user.click(screen.getByRole("button", { name: "重試" }));

    await waitFor(() => {
      expect(actions.declineActionRequest).toHaveBeenCalledTimes(2);
    });
    const calls = vi.mocked(actions.declineActionRequest).mock.calls;
    expect(calls[0]?.[3]).toBe("req-1");
    expect(calls[1]?.[3]).toBe("req-1");
    expect(await screen.findByText("已婉拒 request-1（版本 3）。")).toBeVisible();
    expect(screen.getByRole("button", { name: "開始" })).toBeEnabled();
  });

  it("links Evidence from the completion context, then completes through the exact preview", async () => {
    const user = userEvent.setup();
    const actions = actionsMock();
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([inProgressAction()]))}
        actions={actions}
        newClientRequestId={ids()}
        now={() => NOW}
      />,
    );
    await user.click(await screen.findByRole("button", { name: "連結 Evidence" }));
    expect(actions.loadActionCompletionContext).toHaveBeenCalledWith("action-1");
    await user.selectOptions(await screen.findByLabelText("要連結的 Evidence"), "evidence-1");
    await user.click(screen.getByRole("button", { name: "確認連結" }));
    await waitFor(() => {
      expect(actions.linkActionCompletionEvidence).toHaveBeenCalledWith(
        "action-1",
        2,
        "evidence-1",
        "req-1",
      );
    });
    expect(await screen.findByText(/已將 Evidence evidence-1 連結至 action-1/)).toBeVisible();

    await user.click(screen.getByRole("button", { name: "準備完成" }));
    await user.type(screen.getByLabelText(/Judgment 理由/), "Reviewed in person.");
    await user.selectOptions(screen.getByLabelText("Judgment 分級"), "confidential");
    await user.click(screen.getByRole("button", { name: "產生完成預覽" }));
    await waitFor(() => {
      expect(actions.prepareCompleteAction).toHaveBeenCalledWith(
        "action-1",
        2,
        "Reviewed in person.",
        "confidential",
        "req-2",
      );
    });
    expect(await screen.findByRole("dialog")).toBeVisible();
    expect(screen.getByText("action-1（版本 3）")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "核准並完成" }));
    await waitFor(() => {
      expect(actions.approveAndExecuteCompleteAction).toHaveBeenCalledWith(
        "prepared-complete-1",
        DIGEST,
        "req-3",
      );
    });
    expect(await screen.findByText(/已完成 action-1（版本 4，狀態：已完成）/)).toBeVisible();
  });

  it("prepares a cancel with the person's reason and rejects it durably through the generic reject", async () => {
    const user = userEvent.setup();
    const actions = actionsMock();
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([inProgressAction()]))}
        actions={actions}
        newClientRequestId={ids()}
        now={() => NOW}
      />,
    );
    await user.click(await screen.findByRole("button", { name: "準備取消" }));
    await user.type(screen.getByLabelText("取消理由"), "Scope moved.");
    await user.click(screen.getByRole("button", { name: "產生取消預覽" }));
    await waitFor(() => {
      expect(actions.prepareCancelAction).toHaveBeenCalledWith(
        "action-1",
        2,
        "Scope moved.",
        "req-1",
      );
    });
    await screen.findByRole("dialog");
    expect(screen.getByText("Scope moved.")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "拒絕" }));
    await waitFor(() => {
      expect(actions.rejectPreparedActionIntent).toHaveBeenCalledWith("prepared-cancel-1", "req-3");
    });
    expect(actions.rejectPreparedAcceptActionRequest).not.toHaveBeenCalled();
    expect(await screen.findByText("已拒絕。")).toBeVisible();
  });

  it("reopens a Completed Action in the mode the person chose", async () => {
    const user = userEvent.setup();
    const actions = actionsMock();
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([completedAction()]))}
        actions={actions}
        newClientRequestId={ids()}
        now={() => NOW}
      />,
    );
    await user.click(await screen.findByRole("button", { name: "準備重新開啟" }));
    expect(screen.getByLabelText("重新開啟模式")).toHaveValue("reopen_completed");
    await user.type(screen.getByLabelText("重新開啟理由"), "It came back.");
    await user.click(screen.getByRole("button", { name: "產生重新開啟預覽" }));
    await waitFor(() => {
      expect(actions.prepareReopenAction).toHaveBeenCalledWith(
        "action-1",
        4,
        "reopen_completed",
        "It came back.",
        "req-1",
      );
    });
    await screen.findByRole("dialog");
    await user.click(screen.getByRole("button", { name: "核准並重新開啟" }));
    await waitFor(() => {
      expect(actions.approveAndExecuteReopenAction).toHaveBeenCalledWith(
        "prepared-reopen-1",
        DIGEST,
        "req-2",
      );
    });
    expect(await screen.findByText(/已重新開啟 action-1（版本 5，狀態：待處理）/)).toBeVisible();
  });

  it("resolves a Decision Request through the exact preview, with Evidence and a resulting request", async () => {
    const user = userEvent.setup();
    const actions = actionsMock();
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([openDecisionRequest()]))}
        actions={actions}
        newClientRequestId={ids()}
        now={() => NOW}
      />,
    );
    await user.click(await screen.findByRole("button", { name: "準備解決" }));
    expect(actions.loadEvidenceReferences).toHaveBeenCalledOnce();
    await user.type(screen.getByLabelText("決定內容"), "Proceed with option A.");
    await user.type(screen.getByLabelText("決定理由"), "Best tradeoff.");
    await user.type(screen.getByLabelText("影響"), "On track.");
    await user.click(await screen.findByLabelText(/evidence-1：已驗證/));
    await user.click(screen.getByRole("button", { name: "新增後續 Action Request" }));
    await user.type(screen.getByLabelText("主旨"), "Follow up");
    await user.type(screen.getByLabelText("內容"), "Do the follow-up.");
    await user.type(screen.getByLabelText("負責人（Stakeholder id）"), "owner-1");
    fireEvent.change(screen.getByLabelText("到期日"), { target: { value: "2024-01-15" } });
    await user.click(screen.getByRole("button", { name: "產生解決預覽" }));

    await waitFor(() => {
      expect(actions.prepareResolveDecisionRequest).toHaveBeenCalledWith(
        "decision-request-1",
        2,
        {
          statement: "Proceed with option A.",
          rationale: "Best tradeoff.",
          impact: "On track.",
          evidenceIds: ["evidence-1"],
          judgmentRationale: null,
          judgmentClassification: null,
          resultingActionRequests: [
            {
              subject: "Follow up",
              details: "Do the follow-up.",
              intendedOwner: "owner-1",
              dueAtMillis: Date.parse("2024-01-15T00:00:00Z"),
              classification: "internal",
            },
          ],
        },
        "req-1",
      );
    });
    expect(await screen.findByRole("dialog")).toBeVisible();
    expect(screen.getByText("decision-9")).toBeVisible();
    expect(screen.getByText(/request-9：Follow up/)).toBeVisible();

    await user.click(screen.getByRole("button", { name: "核准並解決" }));
    await waitFor(() => {
      expect(actions.approveAndExecuteResolveDecisionRequest).toHaveBeenCalledWith(
        "prepared-resolve-1",
        DIGEST,
        "req-2",
      );
    });
    expect(
      await screen.findByText(/已建立 Decision decision-9，並建立 1 筆後續 Action Request/),
    ).toBeVisible();
  });

  it("withdraws a Decision Request with a rationale, and rejects a Resolve preview durably", async () => {
    const user = userEvent.setup();
    const actions = actionsMock();
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([openDecisionRequest()]))}
        actions={actions}
        newClientRequestId={ids()}
        now={() => NOW}
      />,
    );
    await user.click(await screen.findByRole("button", { name: "撤回" }));
    await user.type(screen.getByLabelText("撤回理由"), "No longer needed.");
    await user.click(screen.getByRole("button", { name: "確認撤回" }));
    await waitFor(() => {
      expect(actions.withdrawDecisionRequest).toHaveBeenCalledWith(
        "decision-request-1",
        2,
        "No longer needed.",
        "req-1",
      );
    });
    expect(await screen.findByText("已撤回 decision-request-1（版本 3）。")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "準備解決" }));
    await user.type(screen.getByLabelText("決定內容"), "A");
    await user.type(screen.getByLabelText("決定理由"), "B");
    await user.type(screen.getByLabelText("影響"), "C");
    await user.click(screen.getByRole("button", { name: "產生解決預覽" }));
    await screen.findByRole("dialog");
    await user.click(screen.getByRole("button", { name: "拒絕" }));
    await waitFor(() => {
      expect(actions.rejectPreparedDecisionIntent).toHaveBeenCalledWith(
        "prepared-resolve-1",
        "req-4",
      );
    });
    expect(actions.rejectPreparedActionIntent).not.toHaveBeenCalled();
    expect(actions.rejectPreparedAcceptActionRequest).not.toHaveBeenCalled();
  });
  it("prepares an occurrence with no questions and shows the Issue id it will create", async () => {
    const user = userEvent.setup();
    const actions = actionsMock();
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([openRisk()]))}
        actions={actions}
        now={() => NOW}
      />,
    );
    await screen.findByText("Risk risk-1");

    await user.click(screen.getByRole("button", { name: "準備記錄發生" }));

    // No form: the operation carries no words of its own.
    await screen.findByRole("dialog");
    expect(actions.prepareRecordRiskOccurrence).toHaveBeenCalledWith(
      "risk-1",
      1,
      expect.any(String),
    );
    expect(screen.getByText("將建立的 Issue")).toBeInTheDocument();
    expect(screen.getByText("issue-from-occurrence")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "核准並記錄發生" }));
    await screen.findByText(
      /已記錄發生（Risk 現在是版本 \d+），並建立 Issue issue-from-occurrence/,
    );
    expect(actions.approveAndExecuteRecordRiskOccurrence).toHaveBeenCalledTimes(1);
  });

  it("refuses a Risk preview durably, through the Risk namespace", async () => {
    const user = userEvent.setup();
    const actions = actionsMock();
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([openRisk()]))}
        actions={actions}
        now={() => NOW}
      />,
    );
    await screen.findByText("Risk risk-1");

    await user.click(screen.getByRole("button", { name: "準備記錄發生" }));
    await screen.findByRole("dialog");
    await user.click(screen.getByRole("button", { name: "拒絕" }));

    await waitFor(() => {
      expect(actions.rejectPreparedRiskIntent).toHaveBeenCalledTimes(1);
    });
    expect(actions.rejectPreparedIssueIntent).not.toHaveBeenCalled();
    expect(actions.approveAndExecuteRecordRiskOccurrence).not.toHaveBeenCalled();
  });

  it("resolves an Issue only once the person has picked Evidence and said why", async () => {
    const user = userEvent.setup();
    const actions = actionsMock();
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([openIssue()]))}
        actions={actions}
        now={() => NOW}
      />,
    );
    await screen.findByText("Nightly export fails");

    await user.click(screen.getByRole("button", { name: "準備解決" }));
    const confirm = await screen.findByRole("button", { name: "產生解決預覽" });
    // Evidence-gated: the domain refuses a preview with none, so the form
    // does not let one be submitted.
    expect(confirm).toBeDisabled();

    await user.type(screen.getByLabelText("解決理由"), "The export was fixed.");
    expect(confirm).toBeDisabled();
    await user.click(await screen.findByRole("checkbox", { name: /evidence-1/ }));
    expect(confirm).toBeEnabled();

    await user.click(confirm);
    await screen.findByRole("dialog");
    expect(actions.prepareResolveIssue).toHaveBeenCalledWith(
      "issue-1",
      1,
      "resolved",
      "The export was fixed.",
      ["evidence-1"],
      null,
      null,
      expect.any(String),
    );

    await user.click(screen.getByRole("button", { name: "核准並解決" }));
    await screen.findByText(/issue-1 現在是已解決/);
    expect(actions.approveAndExecuteIssueTransition).toHaveBeenCalledTimes(1);
  });

  it("sends an Issue Judgment with its classification, kept apart from the resolution rationale", async () => {
    const user = userEvent.setup();
    const actions = actionsMock();
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([openIssue()]))}
        actions={actions}
        now={() => NOW}
      />,
    );
    await screen.findByText("Nightly export fails");

    await user.click(screen.getByRole("button", { name: "準備解決" }));
    await user.type(await screen.findByLabelText("解決理由"), "The export was fixed.");
    await user.click(await screen.findByRole("checkbox", { name: /evidence-1/ }));
    // No Judgment written: no classification to choose.
    expect(screen.queryByLabelText("Judgment 分級")).not.toBeInTheDocument();
    await user.type(screen.getByLabelText(/^Judgment 理由/), "Read but not pinned yet.");
    const classification = screen.getByLabelText("Judgment 分級");
    expect(classification).toHaveValue("internal");
    await user.selectOptions(classification, "confidential");
    await user.click(screen.getByRole("button", { name: "產生解決預覽" }));

    await screen.findByRole("dialog");
    expect(actions.prepareResolveIssue).toHaveBeenCalledWith(
      "issue-1",
      1,
      "resolved",
      "The export was fixed.",
      ["evidence-1"],
      "Read but not pinned yet.",
      "confidential",
      expect.any(String),
    );
  });

  it("hides the Judgment classification again when the Judgment is cleared", async () => {
    const user = userEvent.setup();
    const actions = actionsMock();
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([openIssue()]))}
        actions={actions}
        now={() => NOW}
      />,
    );
    await screen.findByText("Nightly export fails");

    await user.click(screen.getByRole("button", { name: "準備解決" }));
    await user.type(await screen.findByLabelText("解決理由"), "The export was fixed.");
    await user.click(await screen.findByRole("checkbox", { name: /evidence-1/ }));
    const judgment = await screen.findByLabelText(/^Judgment 理由/);
    await user.type(judgment, "x");
    await user.selectOptions(screen.getByLabelText("Judgment 分級"), "restricted");
    await user.clear(judgment);
    expect(screen.queryByLabelText("Judgment 分級")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "產生解決預覽" }));

    // The classification chosen before clearing must not travel on its own.
    await screen.findByRole("dialog");
    expect(actions.prepareResolveIssue).toHaveBeenCalledWith(
      "issue-1",
      1,
      "resolved",
      "The export was fixed.",
      ["evidence-1"],
      null,
      null,
      expect.any(String),
    );
  });

  it("closes and reopens a Resolved Issue with an optional Judgment on each", async () => {
    const user = userEvent.setup();
    const actions = actionsMock();
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([resolvedIssue()]))}
        actions={actions}
        now={() => NOW}
      />,
    );
    await screen.findByText("Nightly export fails");

    await user.click(screen.getByRole("button", { name: "準備關閉" }));
    await user.click(await screen.findByRole("checkbox", { name: /evidence-1/ }));
    await user.type(screen.getByLabelText(/^Judgment 理由/), "Verified by hand; not pinned.");
    await user.click(screen.getByRole("button", { name: "產生關閉預覽" }));
    await screen.findByRole("dialog");
    expect(actions.prepareCloseIssue).toHaveBeenCalledWith(
      "issue-1",
      2,
      ["evidence-1"],
      "Verified by hand; not pinned.",
      "internal",
      expect.any(String),
    );
    await user.click(screen.getByRole("button", { name: "拒絕" }));
    await waitFor(() => {
      expect(actions.rejectPreparedIssueIntent).toHaveBeenCalledTimes(1);
    });

    await user.click(await screen.findByRole("button", { name: "準備重新開啟" }));
    await user.type(await screen.findByLabelText("重新開啟理由"), "It failed again.");
    await user.click(await screen.findByRole("checkbox", { name: /evidence-1/ }));
    await user.type(
      screen.getByLabelText(/^Judgment 理由/),
      "The failure log was read, not pinned.",
    );
    await user.selectOptions(screen.getByLabelText("Judgment 分級"), "confidential");
    await user.click(screen.getByRole("button", { name: "產生重新開啟預覽" }));
    await screen.findByRole("dialog");
    expect(actions.prepareReopenIssue).toHaveBeenCalledWith(
      "issue-1",
      2,
      "It failed again.",
      ["evidence-1"],
      "The failure log was read, not pinned.",
      "confidential",
      expect.any(String),
    );
  });

  it("refuses an Issue preview durably through the Issue namespace", async () => {
    const user = userEvent.setup();
    const actions = actionsMock();
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([openIssue()]))}
        actions={actions}
        now={() => NOW}
      />,
    );
    await screen.findByText("Nightly export fails");

    await user.click(screen.getByRole("button", { name: "準備解決" }));
    await user.type(await screen.findByLabelText("解決理由"), "The export was fixed.");
    await user.click(await screen.findByRole("checkbox", { name: /evidence-1/ }));
    await user.click(screen.getByRole("button", { name: "產生解決預覽" }));
    await screen.findByRole("dialog");
    await user.click(screen.getByRole("button", { name: "拒絕" }));

    await waitFor(() => {
      expect(actions.rejectPreparedIssueIntent).toHaveBeenCalledTimes(1);
    });
    expect(actions.rejectPreparedRiskIntent).not.toHaveBeenCalled();
  });
});
