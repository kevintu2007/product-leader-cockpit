import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type {
  LensPointDto,
  PortfolioOverviewDto,
  PortfolioRowDto,
  ProductDetailDto,
} from "./cockpitContract";
import { Portfolio } from "./Portfolio";
import { entryTestActions } from "../entry/entryTestActions";
import type { EntryRecordDto } from "../entry/entryIpc";

const NOW = 1_700_000_000_000;
const DAY = 24 * 60 * 60 * 1000;

/** No test here opens the inspector unless it says so. */
function noDetail(): Promise<ProductDetailDto> {
  return new Promise<ProductDetailDto>(() => undefined);
}

function point(productId: string, productName: string): LensPointDto {
  return {
    productId,
    productName,
    productVersion: 1,
    productClassification: "internal",
    timing: {
      state: "datePassed",
      high: true,
      earliestDueAtMillis: NOW - 3 * DAY,
      milestoneCount: 3,
      contributions: [],
    },
    observability: {
      observed: 2,
      defined: 2,
      known: true,
      high: true,
      latestObservedAtMillis: NOW - DAY,
      contributions: [],
    },
    coverage: { verified: 0, linked: 1, byState: [], worst: "unverified", contributions: [] },
    quadrant: "monitorClosely",
    effectiveClassification: "internal",
    classificationForcedBy: null,
    sharedProjectIds: [],
  };
}

function row(productId: string, productName: string, flaggedWorkCount = 0): PortfolioRowDto {
  return { point: point(productId, productName), flaggedWorkCount, classification: "internal" };
}

function overview(overrides: Partial<PortfolioOverviewDto> = {}): PortfolioOverviewDto {
  return {
    state: "success",
    asOfMillis: NOW,
    ledgerRevision: 7,
    dueSoonWindowMillis: 14 * DAY,
    rows: [row("product-1", "Atlas", 2), row("product-2", "Beacon")],
    offset: 0,
    limit: 25,
    total: 2,
    hasMore: false,
    ...overrides,
  };
}

describe("Portfolio", () => {
  it("lists the Products by name and says the order is not a priority", async () => {
    render(<Portfolio loadDetail={noDetail} load={() => Promise.resolve(overview())} />);

    const table = await screen.findByRole("table");
    expect(table.querySelector("caption")).toHaveTextContent("不是優先順序");
    expect(
      within(table)
        .getAllByRole("rowheader")
        .map((header) => header.textContent),
    ).toEqual(["Atlas", "Beacon"]);
  });

  it("shows the Lens measures and the flagged work carried for each Product", async () => {
    render(<Portfolio loadDetail={noDetail} load={() => Promise.resolve(overview())} />);

    const atlas = await screen.findByRole("row", { name: /Atlas/ });
    expect(atlas).toHaveTextContent("已過日期，最早");
    expect(atlas).toHaveTextContent("2／2 個 KPI 有觀測");
    expect(atlas).toHaveTextContent("0／1 份已驗證");
    expect(atlas).toHaveTextContent("2 件");
    expect(screen.getByRole("row", { name: /Beacon/ })).toHaveTextContent("沒有");
  });

  it("labels each row with the folded classification the host sent", async () => {
    render(
      <Portfolio
        loadDetail={noDetail}
        load={() =>
          Promise.resolve(
            overview({ rows: [{ ...row("product-1", "Atlas", 1), classification: "restricted" }] }),
          )
        }
      />,
    );

    expect(await screen.findByRole("row", { name: /Atlas/ })).toHaveTextContent("Restricted");
  });

  it("renders no progress percentage", async () => {
    const { container } = render(
      <Portfolio loadDetail={noDetail} load={() => Promise.resolve(overview())} />,
    );
    await screen.findByRole("table");

    expect(container.textContent).not.toMatch(/%/);
    expect(container.querySelector("progress")).toBeNull();
  });

  it("states the page position rather than implying it", async () => {
    render(
      <Portfolio
        loadDetail={noDetail}
        load={() => Promise.resolve(overview({ offset: 25, total: 60, hasMore: true }))}
      />,
    );

    expect(await screen.findByText(/共 60 個，目前顯示第 26 到 27 個/)).toBeInTheDocument();
  });

  it("advances a page and asks for the next offset", async () => {
    const load = vi.fn((offset: number) =>
      Promise.resolve(overview({ offset, total: 60, hasMore: offset === 0 })),
    );
    render(<Portfolio loadDetail={noDetail} load={load} />);
    await screen.findByRole("table");

    await userEvent.click(screen.getByRole("button", { name: "顯示下一頁" }));

    await waitFor(() => {
      expect(load).toHaveBeenCalledWith(25, 25);
    });
  });

  it("offers no next page when nothing remains", async () => {
    render(<Portfolio loadDetail={noDetail} load={() => Promise.resolve(overview())} />);
    await screen.findByRole("table");

    expect(screen.queryByRole("button", { name: "顯示下一頁" })).toBeNull();
  });

  it("says the Portfolio is empty rather than rendering an empty table", async () => {
    render(
      <Portfolio
        loadDetail={noDetail}
        load={() => Promise.resolve(overview({ rows: [], total: 0 }))}
      />,
    );

    expect(await screen.findByText(/還沒有任何 Product/)).toBeInTheDocument();
    expect(screen.queryByRole("table")).toBeNull();
    expect(screen.queryByText(/第 1 到 0 個/)).toBeNull();
  });

  it("does not call the Ledger empty when only this page is", async () => {
    const load = vi.fn((offset: number) =>
      Promise.resolve(
        offset === 0
          ? overview({ offset: 0, total: 60, hasMore: true })
          : overview({ offset, rows: [], total: 10, hasMore: false }),
      ),
    );
    render(<Portfolio loadDetail={noDetail} load={load} />);
    await screen.findByRole("table");
    await userEvent.click(screen.getByRole("button", { name: "顯示下一頁" }));

    expect(await screen.findByText("這一頁沒有 Product；共有 10 個。")).toBeInTheDocument();
    expect(screen.queryByText(/還沒有任何 Product/)).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "回到第一頁" }));
    await waitFor(() => {
      expect(load).toHaveBeenLastCalledWith(0, 25);
    });
  });

  it("shows nothing but a re-read when the two Ledger reads disagree", async () => {
    render(
      <Portfolio
        loadDetail={noDetail}
        load={() => Promise.resolve(overview({ state: "outOfSync", rows: [], total: 0 }))}
      />,
    );

    await screen.findByText(/版本不一致/);
    expect(screen.getByRole("button", { name: "重新讀取" })).toBeInTheDocument();
    expect(screen.queryByRole("table")).toBeNull();
  });

  it("reports a failed query without showing stale data", async () => {
    render(
      <Portfolio
        loadDetail={noDetail}
        load={() => Promise.reject<PortfolioOverviewDto>(new Error("unavailable"))}
      />,
    );

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("沒有顯示任何舊資料");
    expect(screen.queryByRole("table")).toBeNull();
  });

  it("opens the inspector for the selected Product and keeps the list in place", async () => {
    // DG3 user story 4: selecting a Product updates a persistent inspector so
    // Portfolio context is retained while investigating.
    const loadDetail = vi.fn().mockResolvedValue({
      state: "success",
      asOfMillis: NOW,
      ledgerRevision: 7,
      product: {
        id: "product-1",
        label: "Synthetic Product",
        classification: "internal",
        revision: 1,
        owner: "portfolio",
        asOfMillis: NOW,
      },
      classificationForcedBy: null,
      structure: [],
      evidence: [],
      people: [],
      healthReasons: [],
      impact: "unassessed",
    } satisfies ProductDetailDto);
    render(<Portfolio loadDetail={loadDetail} load={() => Promise.resolve(overview())} />);

    await screen.findByRole("table");
    await userEvent.click(screen.getByRole("button", { name: "檢視 Atlas" }));

    expect(loadDetail).toHaveBeenCalledWith("product-1");
    expect(screen.getByRole("heading", { name: "Atlas 的 Lens 指標" })).toHaveFocus();
    expect(await screen.findByRole("heading", { name: "Synthetic Product" })).toBeInTheDocument();
    // The list is still there.
    expect(screen.getByRole("table")).toBeInTheDocument();
  });

  it("re-reads the page after an inspector write and keeps the selection", async () => {
    // The measures beside the inspector describe the same Evidence the write
    // just changed; leaving them as they were would show two answers at once.
    const verified = overview({
      rows: [
        {
          ...row("product-1", "Atlas", 2),
          point: {
            ...point("product-1", "Atlas"),
            coverage: { verified: 1, linked: 1, byState: [], worst: "verified", contributions: [] },
          },
        },
        row("product-2", "Beacon"),
      ],
    });
    const load = vi
      .fn<(offset: number, limit: number) => Promise<PortfolioOverviewDto>>()
      .mockResolvedValueOnce(overview())
      .mockResolvedValue(verified);
    const loadDetail = (): Promise<ProductDetailDto> =>
      Promise.resolve({
        state: "success",
        asOfMillis: NOW,
        ledgerRevision: 7,
        product: {
          id: "product-1",
          label: "Atlas",
          classification: "internal",
          revision: 1,
          owner: "portfolio",
          asOfMillis: NOW,
        },
        classificationForcedBy: null,
        structure: [],
        evidence: [
          {
            id: "evidence-1",
            role: null,
            verification: "observed_unpinned",
            verifiedAtMillis: null,
            pinned: false,
            classification: "internal",
            classificationAtLink: "internal",
            linkedAtMillis: NOW,
            revision: 3,
          },
        ],
        people: [],
        healthReasons: [],
        impact: "unassessed",
      });
    const evidenceActions = {
      pinEvidenceFingerprint: () =>
        Promise.resolve({
          changed: true,
          evidence: {
            id: "evidence-1",
            role: null,
            verification: { kind: "verified", atMillis: NOW, integrityDigest: null },
            pinned: true,
            classification: "internal" as const,
            version: 4,
          },
          correlationId: "host-1",
        }),
      reobserveEvidenceVerification: () => new Promise<never>(() => undefined),
      loadVaultStatus: () =>
        Promise.resolve({
          configured: true,
          available: true,
          reason: null,
          folderName: null,
          correlationId: "host-0",
        }),
      loadEvidenceReferences: () => new Promise<never>(() => undefined),
      linkEvidenceToProduct: () => new Promise<never>(() => undefined),
    };
    render(
      <Portfolio
        load={() => load(0, 25)}
        loadDetail={loadDetail}
        evidenceActions={evidenceActions}
      />,
    );

    await screen.findByRole("table");
    await userEvent.click(screen.getByRole("button", { name: "檢視 Atlas" }));
    await userEvent.click(await screen.findByRole("tab", { name: "Evidence" }));
    expect(screen.getByRole("complementary")).toHaveTextContent("0／1 份已驗證");

    await userEvent.click(screen.getByRole("button", { name: "釘選指紋" }));
    await userEvent.click(screen.getByRole("button", { name: "確認釘選" }));

    await waitFor(() => {
      expect(load).toHaveBeenCalledTimes(2);
    });
    await waitFor(() => {
      expect(screen.getByRole("complementary")).toHaveTextContent("1／1 份已驗證");
    });
    expect(screen.getByRole("heading", { name: "Atlas 的 Lens 指標" })).toBeInTheDocument();
    expect(screen.getByRole("table")).toBeInTheDocument();
  });
});

describe("Portfolio record entry (slice 6B)", () => {
  const PORTFOLIO: EntryRecordDto = {
    kind: "portfolio",
    id: "portfolio-1",
    name: "Northern",
    details: "The northern line.",
    classification: "internal",
    version: 3,
  };
  const PRODUCT: EntryRecordDto = {
    kind: "product",
    id: "product-2",
    name: "Beacon",
    details: "Beacon.",
    classification: "internal",
    version: 1,
  };

  it("offers nothing to enter without the entry actions", async () => {
    render(<Portfolio loadDetail={noDetail} load={() => Promise.resolve(overview())} />);
    await screen.findByRole("table");
    expect(screen.queryByRole("button", { name: "新增 Portfolio…" })).toBeNull();
    expect(screen.queryByRole("button", { name: "新增 Product…" })).toBeNull();
  });

  it("lists the Portfolios and creates one through the sheet under one request id", async () => {
    const actions = entryTestActions({}, [PORTFOLIO]);
    render(
      <Portfolio
        loadDetail={noDetail}
        load={() => Promise.resolve(overview())}
        entryActions={actions}
      />,
    );
    await screen.findByRole("table");
    expect(await screen.findByText("Northern（Internal，版本 3）")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "新增 Portfolio…" }));
    const dialog = await screen.findByRole("dialog", { name: "新增 Portfolio" });
    await userEvent.type(within(dialog).getByLabelText("名稱"), "Southern");
    await userEvent.type(within(dialog).getByLabelText("說明"), "The southern line.");
    await userEvent.click(within(dialog).getByLabelText("Confidential"));
    await userEvent.click(within(dialog).getByRole("button", { name: "建立" }));

    await waitFor(() => {
      expect(actions.createPortfolio).toHaveBeenCalledWith(
        { name: "Southern", details: "The southern line.", classification: "confidential" },
        expect.stringMatching(/^entry-/),
      );
    });
    expect(await screen.findByRole("status")).toHaveTextContent("已建立 Portfolio portfolio-new。");
    expect(screen.queryByRole("dialog")).toBeNull();
    // Both lists are re-read from the Ledger after the write.
    expect(actions.listEntryRecords).toHaveBeenCalledTimes(2);
  });

  it("edits a Portfolio at the version it read", async () => {
    const actions = entryTestActions({}, [PORTFOLIO]);
    render(
      <Portfolio
        loadDetail={noDetail}
        load={() => Promise.resolve(overview())}
        entryActions={actions}
      />,
    );
    await screen.findByText("Northern（Internal，版本 3）");
    await userEvent.click(screen.getByRole("button", { name: "編輯…" }));
    const dialog = await screen.findByRole("dialog", { name: "編輯 Portfolio" });
    expect(within(dialog).getByLabelText("名稱")).toHaveValue("Northern");
    await userEvent.clear(within(dialog).getByLabelText("名稱"));
    await userEvent.type(within(dialog).getByLabelText("名稱"), "Northern, renamed");
    await userEvent.click(within(dialog).getByRole("button", { name: "儲存" }));
    await waitFor(() => {
      expect(actions.updatePortfolio).toHaveBeenCalledWith(
        "portfolio-1",
        3,
        { name: "Northern, renamed", details: "The northern line.", classification: "internal" },
        expect.stringMatching(/^entry-/),
      );
    });
  });

  it("links a Product to a Portfolio at both versions", async () => {
    const actions = entryTestActions({}, [PORTFOLIO, PRODUCT]);
    render(
      <Portfolio
        loadDetail={noDetail}
        load={() => Promise.resolve(overview())}
        entryActions={actions}
      />,
    );
    await screen.findByText("Northern（Internal，版本 3）");
    await userEvent.click(screen.getByRole("button", { name: "連結一個 Product…" }));
    const dialog = await screen.findByRole("dialog", { name: "把 Product 連結到 Northern" });
    const link = within(dialog).getByRole("button", { name: "連結" });
    expect(link).toBeDisabled();
    await userEvent.selectOptions(within(dialog).getByLabelText("要連結的紀錄"), "product-2");
    // What the link will record: the combine of both ends (§3.3).
    expect(within(dialog).getByText("這個連結會以 Internal 分級記錄。")).toBeInTheDocument();
    await userEvent.click(link);
    await waitFor(() => {
      expect(actions.linkPortfolioProduct).toHaveBeenCalledWith(
        "portfolio-1",
        3,
        "product-2",
        1,
        expect.stringMatching(/^entry-/),
      );
    });
    expect(await screen.findByRole("status")).toHaveTextContent("已連結 relationship-1。");
  });
});
