import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { ExecutiveCockpitDto } from "./cockpitContract";
import { ExecutiveCockpit } from "./ExecutiveCockpit";

function cockpit(overrides: Partial<ExecutiveCockpitDto> = {}): ExecutiveCockpitDto {
  return {
    state: "success",
    asOfMillis: 1_700_000_000_000,
    ledgerRevision: 7,
    periodComparable: false,
    periodNote: "no review period has been approved yet, so there is nothing to compare against",
    pulse: {
      milestones: { count: 0, definition: "Milestones currently tracked", owner: "delivery" },
      commitments: {
        count: 3,
        definition:
          "Accepted Actions. A submitted Action Request is not counted until it is accepted",
        owner: "action_management",
      },
      kpis: { count: 2, definition: "KPIs with a definition in the Ledger", owner: "kpi" },
    },
    products: [
      {
        id: "product-1",
        classification: "internal",
        revision: 7,
        owner: "portfolio",
        degraded: false,
        attentionCount: 1,
      },
    ],
    exceptions: [
      {
        kind: "action_request",
        id: "request-1",
        label: "Approve the Beacon pricing experiment",
        reason: "action_request_response_overdue",
        explanation: "the response deadline has passed",
        tier: "breachedCommitment",
        tierWhy: "a commitment has already been missed",
        relevantAtMillis: 1_699_000_000_000,
        rankRationale:
          "Ranked here because a commitment has already been missed; within that group the earliest time comes first",
        classification: "internal",
        freshness: "fresh",
        degraded: false,
      },
    ],
    leaderConclusion: "1 item needs attention.",
    leaderIntervention: "Ranked here because a commitment has already been missed",
    lens: {
      asOfMillis: 1_700_000_000_000,
      ledgerRevision: 7,
      dueSoonWindowMillis: 14 * 24 * 60 * 60 * 1000,
      points: [],
    },
    ...overrides,
  };
}

describe("ExecutiveCockpit", () => {
  it("renders the DG1 information sequence in its fixed order", async () => {
    render(<ExecutiveCockpit load={() => Promise.resolve(cockpit())} />);

    // Portfolio health first -- the Lens -- then period, pulse, attention
    // and the briefing.
    const headings = await screen.findAllByRole("heading", { level: 3 });
    expect(headings.map((heading) => heading.textContent)).toEqual([
      "Portfolio Lens",
      "期間比較",
      "Portfolio 現況",
      "需要注意的事項",
      "給你的摘要",
    ]);
  });

  it("shows every pulse count with the definition that makes it inspectable", async () => {
    render(<ExecutiveCockpit load={() => Promise.resolve(cockpit())} />);

    expect(
      await screen.findByText("已接受的 Action。送出但還沒被接受的 Action Request 不算在內"),
    ).toBeInTheDocument();
    expect(screen.getByText("在 Ledger 中有定義的 KPI 數")).toBeInTheDocument();
    expect(screen.getByText("來自Action 管理")).toBeInTheDocument();
  });

  it("renders no progress percentage anywhere", async () => {
    // DG1 forbids an unsupported progress percentage. The composition carries
    // counts only, so there is nothing to render -- this guards against a
    // future edit inventing one in the view layer.
    const { container } = render(<ExecutiveCockpit load={() => Promise.resolve(cockpit())} />);
    await screen.findByText("Portfolio 現況");

    expect(container.textContent).not.toMatch(/%/);
    expect(container.querySelector("progress")).toBeNull();
    expect(container.querySelector('[role="progressbar"]')).toBeNull();
  });

  it("states that no period comparison is possible rather than showing a zero delta", async () => {
    render(<ExecutiveCockpit load={() => Promise.resolve(cockpit())} />);

    // Scoped to the period section: a bare "0" legitimately appears as a
    // pulse count elsewhere, and asserting against the whole document would
    // have been testing the wrong thing.
    const period = (await screen.findByRole("heading", { name: "期間比較" })).closest("section");
    expect(period).toHaveTextContent(/無法比較/);
    expect(period?.textContent).not.toMatch(/[0-9]/);
  });

  it("shows why each attention item is ranked where it is", async () => {
    render(<ExecutiveCockpit load={() => Promise.resolve(cockpit())} />);

    const attention = (await screen.findByRole("heading", { name: "需要注意的事項" })).closest(
      "section",
    );
    expect(attention).toHaveTextContent("排在這裡：承諾已經落空");
  });

  it("names the record each exception is about, in words rather than identifiers", async () => {
    // Before the fix the Cockpit could give a reason but never say which
    // record it was about.
    render(<ExecutiveCockpit load={() => Promise.resolve(cockpit())} />);

    const attention = (await screen.findByRole("heading", { name: "需要注意的事項" })).closest(
      "section",
    );
    expect(attention).toHaveTextContent("Action Request");
    expect(attention).toHaveTextContent("Approve the Beacon pricing experiment");
    expect(attention).toHaveTextContent("回覆期限已經過了");
    expect(attention).not.toHaveTextContent("action_request_response_overdue");
  });

  it("shows nothing but a re-read when the two Ledger reads disagree", async () => {
    render(
      <ExecutiveCockpit
        load={() =>
          Promise.resolve(
            cockpit({ state: "outOfSync", pulse: null, products: [], exceptions: [], lens: null }),
          )
        }
      />,
    );

    // Wait for the answer itself: the loading message is a status too.
    await screen.findByText(/版本不一致/);
    expect(screen.getByRole("status")).toHaveTextContent("版本不一致");
    expect(screen.getByRole("button", { name: "重新讀取" })).toBeInTheDocument();
    // Not a blend of two moments: no section of the Cockpit is rendered.
    expect(screen.queryByRole("heading", { name: "Portfolio 現況" })).toBeNull();
    expect(screen.queryByText("目前沒有需要注意的事項。")).toBeNull();
  });

  it("reports a failed query without showing stale data", async () => {
    // A failed query must not fall back to a previous answer presented as
    // current, and must not change state.
    render(
      <ExecutiveCockpit
        load={() => Promise.reject<ExecutiveCockpitDto>(new Error("unavailable"))}
      />,
    );

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("沒有顯示任何舊資料");
    expect(screen.queryByText("Portfolio 現況")).toBeNull();
  });

  it("renders the host's safe error envelope with its correlation id, never the raw key", async () => {
    render(
      <ExecutiveCockpit
        load={() =>
          // The host rejects with its safe envelope, not an Error.
          // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors
          Promise.reject<ExecutiveCockpitDto>({
            errorCode: "PLATFORM_INTERNAL",
            messageKey: "desktop.snapshot_unavailable",
            messageParams: [],
            correlationId: "host-1a2b-7",
            retryable: true,
            extensions: [],
          })
        }
      />,
    );

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("目前無法讀取 Product Ledger 的快照");
    expect(alert).toHaveTextContent("host-1a2b-7");
    expect(alert).not.toHaveTextContent("desktop.snapshot_unavailable");
    // The route keeps its own re-read control regardless of retryability.
    expect(screen.getByRole("button", { name: "重新讀取" })).toBeInTheDocument();
  });

  it("lets a keyboard user retry a failed query", async () => {
    const load = vi
      .fn()
      .mockRejectedValueOnce(new Error("unavailable"))
      .mockResolvedValueOnce(cockpit()) as () => Promise<ExecutiveCockpitDto>;
    render(<ExecutiveCockpit load={load} />);
    await screen.findByRole("alert");

    await userEvent.tab();
    expect(screen.getByRole("button", { name: "重新讀取" })).toHaveFocus();
    await userEvent.keyboard("{Enter}");

    await waitFor(() => {
      expect(screen.getByText("Portfolio 現況")).toBeInTheDocument();
    });
  });

  it("counts records, not reasons, in the briefing", async () => {
    const [first] = cockpit().exceptions;
    if (first === undefined) {
      throw new Error("fixture has an exception");
    }
    render(
      <ExecutiveCockpit
        load={() =>
          Promise.resolve(
            cockpit({
              exceptions: [first, { ...first, reason: "action_request_without_owner" }],
            }),
          )
        }
      />,
    );

    const briefing = (await screen.findByRole("heading", { name: "給你的摘要" })).closest(
      "section",
    );
    expect(briefing).toHaveTextContent("有 1 件事需要注意");
    expect(briefing).toHaveTextContent("最前面的是「Approve the Beacon pricing experiment」");
    expect(briefing).toHaveTextContent("它排在最前面，因為承諾已經落空");
  });

  it("says nothing needs attention rather than rendering an empty list", async () => {
    render(
      <ExecutiveCockpit
        load={() => Promise.resolve(cockpit({ exceptions: [], leaderIntervention: null }))}
      />,
    );

    expect(await screen.findByText("目前沒有需要注意的事項。")).toBeInTheDocument();
    expect(screen.queryByRole("list", { name: /注意/ })).toBeNull();
  });

  it("announces loading through a live region", () => {
    // Never settles, so the route stays in its loading state.
    const pending = new Promise<ExecutiveCockpitDto>(() => undefined);
    render(<ExecutiveCockpit load={() => pending} />);

    expect(screen.getByRole("status")).toHaveTextContent("正在讀取");
  });
});
