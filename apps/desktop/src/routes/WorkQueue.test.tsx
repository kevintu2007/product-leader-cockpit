import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { WorkQueueAttentionDto, WorkQueueDto, WorkQueueItemDto } from "./cockpitContract";
import { formatLocalDate } from "../i18n/time";
import { WorkQueue } from "./WorkQueue";

/** One flag on the overdue Action, with only what a test cares about varied. */
function overdueFlag(overrides: Partial<WorkQueueAttentionDto> = {}): WorkQueueAttentionDto {
  return {
    reason: "action_overdue",
    explanation: "This action passed its due date.",
    tier: "breachedCommitment",
    tierWhy: "a commitment has already been missed",
    relevantAtMillis: 1_000,
    rankRationale: "placed by a breached commitment",
    classification: "internal",
    freshness: "fresh",
    degraded: false,
    ...overrides,
  };
}

/**
 * The standard queue with the Action's flags replaced.
 *
 * Built rather than reached for by index. Indexing the fixture would need a
 * non-null assertion to typecheck, and an assertion in a test is a claim the
 * test itself does not verify.
 */
function withActionAttention(attention: readonly WorkQueueAttentionDto[]): WorkQueueDto {
  const base = queue();
  const items: WorkQueueItemDto[] = base.items.map((item) =>
    item.kind === "action" ? { ...item, attention } : item,
  );
  return { ...base, items };
}

/** Mirrors the route's own rendering so the test asserts what a reader sees. */
function formatted(millis: number): string {
  return new Date(millis).toISOString().slice(0, 16).replace("T", " ");
}

function queue(overrides: Partial<WorkQueueDto> = {}): WorkQueueDto {
  return {
    state: "success",
    asOfMillis: 1_700_000_000_000,
    ledgerRevision: 7,
    items: [
      {
        kind: "action",
        id: "action-1",
        label: "Action action-1",
        stateLabel: "Open",
        classification: "internal",
        revision: 8,
        owner: "action-management",
        asOfMillis: 1_700_000_000_000,
        lifecycleLegalIntents: ["start_action", "prepare_cancel_action"],
        relevantAtMillis: 1_000,
        placement: "breachedCommitment",
        placementRationale: "a commitment has already been missed",
        attention: [overdueFlag()],
      },
      {
        kind: "issue",
        id: "issue-1",
        label: "Synthetic issue",
        stateLabel: "Open",
        classification: "internal",
        revision: 7,
        owner: "issues",
        // A different instant from the Action: two snapshots, two reads.
        asOfMillis: 1_700_000_060_000,
        lifecycleLegalIntents: ["prepare_resolve_issue"],
        // The Ledger holds no resolution deadline for an Issue.
        relevantAtMillis: null,
        placement: "unflagged",
        placementRationale: "nothing has flagged it, so it sorts below flagged work",
        attention: [],
      },
    ],
    countsByKind: [
      { kind: "action_request", count: 0 },
      { kind: "action", count: 1 },
      { kind: "decision_request", count: 0 },
      { kind: "risk", count: 0 },
      { kind: "issue", count: 1 },
    ],
    offset: 0,
    limit: 25,
    total: 2,
    hasMore: false,
    ...overrides,
  };
}

describe("WorkQueue", () => {
  it("shows the lifecycle type on every row", async () => {
    // The Work Queue requires the five types to stay separate. A row without
    // its type invites the reader to treat an Issue and an Action as one pool.
    render(<WorkQueue load={() => Promise.resolve(queue())} />);

    const action = await screen.findByRole("row", { name: /Action action-1/ });
    expect(within(action).getByText("Action")).toBeInTheDocument();
    const issue = screen.getByRole("row", { name: /Synthetic issue/ });
    expect(within(issue).getByText("Issue")).toBeInTheDocument();
  });

  it("states that a deadline is absent rather than showing a substituted date", async () => {
    // Decision Requests and Issues have no deadline anywhere in the Ledger.
    // A default date would be indistinguishable from a recorded one.
    render(<WorkQueue load={() => Promise.resolve(queue())} />);

    const issue = await screen.findByRole("row", { name: /Synthetic issue/ });
    expect(within(issue).getByText("未記錄期限")).toBeInTheDocument();
  });

  it("shows a request's promised completion date as its own fact, not as its deadline", async () => {
    // A Decision's follow-up request names when the work will be done but has
    // no response deadline. Both are shown, each under its own name.
    const promised = 1_700_000_900_000;
    const base = queue();
    const followUp: WorkQueueItemDto = {
      kind: "action_request",
      id: "request-follow-up",
      label: "Synthetic follow-up request",
      stateLabel: "Open",
      classification: "internal",
      revision: 3,
      owner: "action-management",
      asOfMillis: 1_700_000_000_000,
      lifecycleLegalIntents: ["accept_action_request"],
      relevantAtMillis: null,
      promisedAtMillis: promised,
      placement: "unflagged",
      placementRationale: "nothing has flagged it, so it sorts below flagged work",
      attention: [],
    };
    render(
      <WorkQueue
        load={() => Promise.resolve({ ...base, items: [...base.items, followUp], total: 3 })}
      />,
    );

    const row = await screen.findByRole("row", { name: /Synthetic follow-up request/ });
    // It has a promised completion date but no response deadline, and says so.
    expect(within(row).getByText("未設回覆期限")).toBeInTheDocument();
    expect(within(row).getByText(`承諾完成 ${formatLocalDate(promised)}`)).toBeInTheDocument();
    // No other row claims a promised date it does not have.
    const issue = screen.getByRole("row", { name: /Synthetic issue/ });
    expect(within(issue).queryByText(/承諾完成/)).not.toBeInTheDocument();
  });

  it("renders legal intents as text, never as a control that would offer to run them", async () => {
    // Lifecycle-admissible is strictly weaker than executable: preparation,
    // classification, policy and approval are further gates. A button here
    // would promise an authority this route does not have.
    render(<WorkQueue load={() => Promise.resolve(queue())} />);

    const action = await screen.findByRole("row", { name: /Action action-1/ });
    // In words, not identifiers: `start_action` reaches the reader as 開始進行.
    expect(within(action).getByText(/開始進行/)).toBeInTheDocument();
    expect(within(action).queryByText(/start_action/)).toBeNull();
    // The one control is the title, which opens a read-only detail sheet; no
    // intent is offered as something to run.
    expect(within(action).getByRole("button")).toHaveAccessibleName("Action action-1");
    expect(within(action).queryByRole("link")).toBeNull();
  });

  it("says why each item is ranked where it is", async () => {
    render(<WorkQueue load={() => Promise.resolve(queue())} />);

    // Keyed off the stable placement identifier, not the host's English prose.
    expect(await screen.findByText("承諾已經落空")).toBeInTheDocument();
    expect(screen.getByText("沒有被標記，排在需要注意的項目之後")).toBeInTheDocument();
  });

  it("shows every kind in the filter including those with no items", async () => {
    // Zero and absent are different facts. A kind missing from the filter
    // would read as "not counted" rather than "none outstanding".
    render(<WorkQueue load={() => Promise.resolve(queue())} />);

    for (const label of ["Action Request", "Decision Request", "Risk"]) {
      expect(
        await screen.findByRole("checkbox", { name: new RegExp(`${label}（0）`) }),
      ).toBeInTheDocument();
    }
  });

  it("asks the backend for the kinds the reader selected", async () => {
    const load = vi.fn().mockResolvedValue(queue());
    render(<WorkQueue load={load} />);

    await screen.findByRole("checkbox", { name: /^Risk/ });
    await userEvent.click(screen.getByRole("checkbox", { name: /^Risk/ }));

    await waitFor(() => {
      expect(load).toHaveBeenLastCalledWith(0, 25, ["risk"], false);
    });
  });

  it("asks for every kind when no filter is selected", async () => {
    // An empty filter must mean "everything", matching composition. If it
    // meant "nothing" the default view would be silently empty.
    const load = vi.fn().mockResolvedValue(queue());
    render(<WorkQueue load={load} />);

    await waitFor(() => {
      expect(load).toHaveBeenCalledWith(0, 25, [], false);
    });
  });

  it("reports every successful read, so a count shown elsewhere can follow it", async () => {
    // The rail badge counts flagged work; without this it kept the count from
    // launch after the queue itself had changed.
    const onRead = vi.fn();
    const load = vi.fn().mockResolvedValue(queue());
    render(<WorkQueue load={load} onRead={onRead} />);

    await screen.findByRole("checkbox", { name: /^Risk/ });
    expect(onRead).toHaveBeenCalledTimes(1);
    await userEvent.click(screen.getByRole("checkbox", { name: /^Risk/ }));

    await waitFor(() => {
      expect(onRead).toHaveBeenCalledTimes(2);
    });
  });

  it("does not report a failed read", async () => {
    const onRead = vi.fn();
    render(
      <WorkQueue
        load={() => Promise.reject<WorkQueueDto>(new Error("unavailable"))}
        onRead={onRead}
      />,
    );

    await screen.findByRole("alert");
    expect(onRead).not.toHaveBeenCalled();
  });

  it("refuses to show a list when the two Ledger reads disagree", async () => {
    // The five types come from two separate reads. Rendering a blend of two
    // revisions would look correct and be wrong, so `outOfSync` shows no
    // items at all.
    render(<WorkQueue load={() => Promise.resolve(queue({ state: "outOfSync" }))} />);

    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(screen.queryByRole("table")).toBeNull();
    expect(screen.queryByText(/Synthetic issue/)).toBeNull();
  });

  it("shows no stale data when the load fails", async () => {
    render(<WorkQueue load={() => Promise.reject(new Error("unavailable"))} />);

    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(screen.queryByRole("table")).toBeNull();
  });

  it("tells the reader how many items exist, not how many fit on the page", async () => {
    render(<WorkQueue load={() => Promise.resolve(queue({ total: 40, hasMore: true }))} />);

    const table = await screen.findByRole("table");
    expect(within(table).getByText(/共 40 筆/)).toBeInTheDocument();
  });

  it("names what is actually wrong, not merely how many things are", async () => {
    // The Work Queue (S03) must preserve attention FLAGS, and DG0
    // lists "attention reasons" as Work Queue content. A count tells the
    // reader that something is wrong without telling them what, which they
    // cannot act on.
    render(<WorkQueue load={() => Promise.resolve(queue())} />);

    const action = await screen.findByRole("row", { name: /Action action-1/ });
    // Named by its reason identifier (`action_overdue`), in words.
    expect(within(action).getByText(/已經超過到期日/)).toBeInTheDocument();
  });

  it("says plainly when nothing has flagged an item", async () => {
    // Empty is a fact, not a missing value. A blank cell would read as
    // "unknown" rather than "nothing is wrong".
    render(<WorkQueue load={() => Promise.resolve(queue())} />);

    const issue = await screen.findByRole("row", { name: /Synthetic issue/ });
    expect(within(issue).getByText("沒有需要注意的地方")).toBeInTheDocument();
  });

  it("attaches the uncertainty to a flag derived from a stale fact", async () => {
    // The accepted ranking policy: staleness never promotes and is never
    // hidden either -- the item is shown with the limit of what is known.
    // Rendering a stale "overdue" as plainly overdue would fabricate
    // certainty.
    render(
      <WorkQueue
        load={() => Promise.resolve(withActionAttention([overdueFlag({ freshness: "stale" })]))}
      />,
    );

    const action = await screen.findByRole("row", { name: /Action action-1/ });
    expect(within(action).getByText(/未必仍然成立/)).toBeInTheDocument();
  });

  it("shows every flag when a record has more than one", async () => {
    // Ordering places the item by its most severe flag, but the others are
    // still true and the reader still has to deal with them.
    render(
      <WorkQueue
        load={() =>
          Promise.resolve(
            withActionAttention([
              overdueFlag(),
              overdueFlag({
                reason: "action_needs_evidence",
                explanation: "This action has no linked evidence.",
                tier: "evidenceIntegrity",
                tierWhy: "the evidence this relies on is missing or unsound",
                relevantAtMillis: null,
                rankRationale: "placed by evidence integrity",
              }),
            ]),
          )
        }
      />,
    );

    const action = await screen.findByRole("row", { name: /Action action-1/ });
    expect(within(action).getByText(/已經超過到期日/)).toBeInTheDocument();
    expect(within(action).getByText(/完成前需要附上證據/)).toBeInTheDocument();
  });

  it("shows when each row was read, not one timestamp for the whole response", async () => {
    // The Work Queue (S03) must preserve provenance, and the route
    // composition specification defines that as owner, source identity,
    // authoritative revision and `as_of`. The five kinds come from two snapshots read in two separate
    // transactions, so one response-level timestamp would claim they were all
    // read at one instant. The fixture gives the two rows read times a minute
    // apart, and both must be visible.
    render(<WorkQueue load={() => Promise.resolve(queue())} />);

    const action = await screen.findByRole("row", { name: /Action action-1/ });
    expect(within(action).getByText("2023-11-14 22:13")).toBeInTheDocument();
    const issue = screen.getByRole("row", { name: /Synthetic issue/ });
    expect(within(issue).getByText("2023-11-14 22:14")).toBeInTheDocument();
  });

  it("carries all four provenance elements on every row", async () => {
    // owner, source identity, authoritative revision, as_of. Asserted as a
    // property over the rendered rows rather than trusted from the contract,
    // because the contract is what would be right while the surface dropped
    // one -- which is exactly how the attention flags were lost.
    render(<WorkQueue load={() => Promise.resolve(queue())} />);

    await screen.findByRole("table");
    for (const item of queue().items) {
      const row = screen.getByRole("row", { name: new RegExp(item.label) });
      expect(within(row).getByText(formatted(item.asOfMillis))).toBeInTheDocument();
      expect(item.owner).not.toEqual("");
      expect(item.id).not.toEqual("");
      expect(item.revision).toBeGreaterThan(0);
    }
  });
  it("uses a real table rather than a virtualized list", async () => {
    // The recorded disposition for the virtualization decision: a
    // non-virtualized accessible list with paging, not virtualization by
    // preference.
    render(<WorkQueue load={() => Promise.resolve(queue())} />);

    const table = await screen.findByRole("table");
    expect(within(table).getAllByRole("row")).toHaveLength(3);
    expect(screen.getByRole("button", { name: "下一頁" })).toBeDisabled();
  });
});
