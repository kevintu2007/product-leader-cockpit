import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { EntryOutcomeDto } from "../entry/entryIpc";
import { entryTestActions, outcome } from "../entry/entryTestActions";
import type { WorkQueueDto, WorkQueueItemDto } from "./cockpitContract";
import { WorkQueue, type WorkQueueActions } from "./WorkQueue";

const NOW = 1_700_000_000_000;

function item(overrides: Partial<WorkQueueItemDto>): WorkQueueItemDto {
  return {
    kind: "issue",
    id: "issue-1",
    label: "Synthetic issue",
    stateLabel: "Open",
    classification: "internal",
    revision: 1,
    owner: "issues",
    asOfMillis: NOW,
    lifecycleLegalIntents: [],
    relevantAtMillis: null,
    placement: "unflagged",
    placementRationale: "nothing has flagged it",
    attention: [],
    ...overrides,
  };
}

function draftRequest(): WorkQueueItemDto {
  return item({
    kind: "action_request",
    id: "request-1",
    label: "Action Request request-1",
    stateLabel: "Draft",
    revision: 3,
    owner: "action-management",
    lifecycleLegalIntents: ["submit_action_request"],
  });
}

function openRisk(): WorkQueueItemDto {
  return item({
    kind: "risk",
    id: "risk-1",
    label: "Risk risk-1",
    revision: 4,
    owner: "risks",
    lifecycleLegalIntents: ["prepare_record_risk_occurrence", "prepare_close_risk"],
  });
}

function queue(items: readonly WorkQueueItemDto[]): WorkQueueDto {
  return {
    state: "success",
    asOfMillis: NOW,
    ledgerRevision: 7,
    items,
    countsByKind: [
      { kind: "action_request", count: 0 },
      { kind: "action", count: 0 },
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

/** The row commands are not under test here; a host that offers none. */
const NO_ROW_COMMANDS = {} as WorkQueueActions;

const STAKEHOLDERS = [
  {
    kind: "stakeholder",
    id: "stakeholder-a",
    name: "王小明",
    stakeholderKind: "person",
    classification: "internal",
    version: 1,
  },
] as const;

describe("WorkQueue record entry (slice 6E)", () => {
  it("offers nothing to enter without the entry actions", async () => {
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([draftRequest(), openRisk()]))}
        actions={NO_ROW_COMMANDS}
      />,
    );
    await screen.findByRole("table");
    expect(screen.queryByRole("button", { name: "新增 Action Request…" })).toBeNull();
    expect(screen.queryByRole("button", { name: "送出" })).toBeNull();
    expect(screen.queryByRole("button", { name: "更新應對…" })).toBeNull();
  });

  it("creates an Action Request draft naming a Stakeholder read as its owner", async () => {
    const actions = entryTestActions({}, STAKEHOLDERS);
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([]))}
        actions={NO_ROW_COMMANDS}
        entryActions={actions}
        timeZone="Asia/Taipei"
      />,
    );
    await userEvent.click(await screen.findByRole("button", { name: "新增 Action Request…" }));
    const dialog = await screen.findByRole("dialog", { name: "新增 Action Request（草稿）" });
    expect(actions.listEntryRecords).toHaveBeenCalledWith("stakeholder");
    fireEvent.change(within(dialog).getByLabelText("標題"), {
      target: { value: "確認供應商交期" },
    });
    fireEvent.change(within(dialog).getByLabelText("說明"), { target: { value: "請於本週回覆" } });
    await userEvent.selectOptions(within(dialog).getByLabelText("預定負責人"), "stakeholder-a");
    fireEvent.change(within(dialog).getByLabelText("回覆期限"), {
      target: { value: "2026-09-30T09:00" },
    });
    // A request never offers Unclassified: the domain would refuse it at submit.
    expect(within(dialog).queryByLabelText("Unclassified")).toBeNull();
    await userEvent.click(within(dialog).getByLabelText("Internal"));
    await userEvent.click(within(dialog).getByRole("button", { name: "建立" }));
    await waitFor(() => {
      expect(actions.createActionRequestDraft).toHaveBeenCalledWith(
        {
          title: "確認供應商交期",
          details: "請於本週回覆",
          intendedOwnerId: "stakeholder-a",
          responseDueAtMillis: Date.UTC(2026, 8, 30, 1, 0),
          intendedActionDueAtMillis: null,
          classification: "internal",
        },
        expect.stringMatching(/^entry-/),
      );
    });
    expect(await screen.findByRole("status")).toHaveTextContent(
      "已建立 Action Request request-new。",
    );
  });

  it("submits a draft at the version the row read and re-reads the queue", async () => {
    const actions = entryTestActions();
    const load = vi.fn(() => Promise.resolve(queue([draftRequest()])));
    render(<WorkQueue load={load} actions={NO_ROW_COMMANDS} entryActions={actions} />);
    const row = await screen.findByRole("row", { name: /Action Request request-1/ });
    await userEvent.click(within(row).getByRole("button", { name: "送出" }));
    await waitFor(() => {
      expect(actions.submitActionRequest).toHaveBeenCalledWith(
        "request-1",
        3,
        expect.stringMatching(/^entry-/),
      );
    });
    expect(await screen.findByRole("status")).toHaveTextContent(
      "已送出 Action Request request-1：現在是開放狀態。",
    );
    await waitFor(() => {
      expect(load).toHaveBeenCalledTimes(2);
    });
  });

  it("offers no second write while a submit is in flight", async () => {
    // §3.6: one authoritative write at a time from this surface. A pending
    // submit must close the New… controls too, not only the row's own buttons.
    let finish: (() => void) | undefined;
    const actions = entryTestActions({
      submitActionRequest: vi.fn(
        (id: string) =>
          new Promise<EntryOutcomeDto>((resolve) => {
            finish = () => {
              resolve(outcome("action_request", id, 4));
            };
          }),
      ),
    });
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([draftRequest()]))}
        actions={NO_ROW_COMMANDS}
        entryActions={actions}
      />,
    );
    const row = await screen.findByRole("row", { name: /Action Request request-1/ });
    await userEvent.click(within(row).getByRole("button", { name: "送出" }));
    expect(screen.getByRole("button", { name: "新增 Action Request…" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "新增 Risk…" })).toBeDisabled();
    expect(within(row).getByRole("button", { name: "送出" })).toBeDisabled();

    finish?.();
    await screen.findByRole("status");
    expect(screen.getByRole("button", { name: "新增 Action Request…" })).toBeEnabled();
  });

  it("refuses to accept a Risk before its owner, rationale, exposure and review are all named", async () => {
    const actions = entryTestActions({}, STAKEHOLDERS);
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([openRisk()]))}
        actions={NO_ROW_COMMANDS}
        entryActions={actions}
        timeZone="Asia/Taipei"
      />,
    );
    const row = await screen.findByRole("row", { name: /Risk risk-1/ });
    await userEvent.click(within(row).getByRole("button", { name: "更新應對…" }));
    const dialog = await screen.findByRole("dialog", { name: "Risk risk-1 的應對" });
    await userEvent.selectOptions(within(dialog).getByLabelText("應對方式"), "accept");
    expect(
      within(dialog).getByText("接受或轉移 Risk 需要負責人、理由、殘餘曝險與下次檢視時間。"),
    ).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "儲存" })).toBeDisabled();

    await userEvent.selectOptions(within(dialog).getByLabelText("負責人"), "stakeholder-a");
    fireEvent.change(within(dialog).getByLabelText("理由"), { target: { value: "成本可接受" } });
    fireEvent.change(within(dialog).getByLabelText("殘餘曝險"), { target: { value: "低" } });
    fireEvent.change(within(dialog).getByLabelText("下次檢視"), {
      target: { value: "2026-12-01T10:00" },
    });
    await userEvent.click(within(dialog).getByRole("button", { name: "儲存" }));
    await waitFor(() => {
      expect(actions.updateRiskResponse).toHaveBeenCalledWith(
        "risk-1",
        4,
        {
          response: "accept",
          ownerId: "stakeholder-a",
          rationale: "成本可接受",
          residualExposure: "低",
          nextReviewAtMillis: Date.UTC(2026, 11, 1, 2, 0),
        },
        expect.stringMatching(/^entry-/),
      );
    });
    expect(await screen.findByRole("status")).toHaveTextContent("已更新 Risk risk-1 的應對。");
  });

  it("sends a mitigation with nothing else named, as the domain allows", async () => {
    const actions = entryTestActions();
    render(
      <WorkQueue
        load={() => Promise.resolve(queue([openRisk()]))}
        actions={NO_ROW_COMMANDS}
        entryActions={actions}
      />,
    );
    const row = await screen.findByRole("row", { name: /Risk risk-1/ });
    await userEvent.click(within(row).getByRole("button", { name: "更新應對…" }));
    const dialog = await screen.findByRole("dialog", { name: "Risk risk-1 的應對" });
    await userEvent.selectOptions(within(dialog).getByLabelText("應對方式"), "mitigate");
    await userEvent.click(within(dialog).getByRole("button", { name: "儲存" }));
    await waitFor(() => {
      expect(actions.updateRiskResponse).toHaveBeenCalledWith(
        "risk-1",
        4,
        {
          response: "mitigate",
          ownerId: null,
          rationale: null,
          residualExposure: null,
          nextReviewAtMillis: null,
        },
        expect.stringMatching(/^entry-/),
      );
    });
  });
});
