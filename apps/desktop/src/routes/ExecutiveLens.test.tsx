import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type {
  ExecutiveCockpitDto,
  ExecutiveLensDto,
  LensPointDto,
  ProductDetailDto,
} from "./cockpitContract";
import { ExecutiveCockpit } from "./ExecutiveCockpit";

const NOW = 1_700_000_000_000;
const DAY = 24 * 60 * 60 * 1000;

function point(overrides: Partial<LensPointDto> & Pick<LensPointDto, "productId">): LensPointDto {
  return {
    productName: overrides.productId,
    productVersion: 1,
    productClassification: "internal",
    timing: {
      state: "dueSoon",
      high: true,
      earliestDueAtMillis: NOW + 3 * DAY,
      milestoneCount: 2,
      contributions: [
        {
          kind: "relationship",
          id: "rel-1",
          version: 1,
          classification: "internal",
          role: "project_product",
        },
      ],
    },
    observability: {
      observed: 1,
      defined: 3,
      known: true,
      high: false,
      latestObservedAtMillis: NOW - DAY,
      contributions: [],
    },
    coverage: {
      verified: 1,
      linked: 2,
      byState: [
        { state: "unverified", count: 1 },
        { state: "verified", count: 1 },
      ],
      worst: "unverified",
      contributions: [
        {
          kind: "evidence_link",
          id: "evidence-1 -> product x",
          version: null,
          classification: "internal",
          role: "evidence_link",
        },
      ],
    },
    quadrant: "prioritizeNow",
    effectiveClassification: "internal",
    classificationForcedBy: null,
    sharedProjectIds: [],
    ...overrides,
  };
}

const beacon = point({ productId: "product-beacon", productName: "Beacon" });
const atlas = point({
  productId: "product-atlas",
  productName: "Atlas",
  timing: {
    state: "unknown",
    high: null,
    earliestDueAtMillis: null,
    milestoneCount: 0,
    contributions: [],
  },
  observability: {
    observed: 0,
    defined: 0,
    known: false,
    high: null,
    latestObservedAtMillis: null,
    contributions: [],
  },
  coverage: { verified: 0, linked: 0, byState: [], worst: null, contributions: [] },
  quadrant: null,
});

function lens(points: LensPointDto[]): ExecutiveLensDto {
  return { asOfMillis: NOW, ledgerRevision: 9, dueSoonWindowMillis: 14 * DAY, points };
}

function cockpit(points: LensPointDto[]): ExecutiveCockpitDto {
  return {
    state: "success",
    asOfMillis: NOW,
    ledgerRevision: 9,
    periodComparable: false,
    periodNote: "no review period has been approved yet, so there is nothing to compare against",
    pulse: {
      milestones: { count: 2, definition: "d", owner: "delivery" },
      commitments: { count: 0, definition: "d", owner: "action_management" },
      kpis: { count: 3, definition: "d", owner: "kpi" },
    },
    products: [],
    exceptions: [],
    leaderConclusion: "",
    leaderIntervention: null,
    lens: lens(points),
  };
}

function renderCockpit(points: LensPointDto[], loadDetail?: () => Promise<ProductDetailDto>) {
  return render(
    <ExecutiveCockpit
      load={() => Promise.resolve(cockpit(points))}
      {...(loadDetail === undefined ? {} : { loadDetail })}
    />,
  );
}

describe("the Executive Lens", () => {
  it("places every Product and lists them in a table that says it is not a ranking", async () => {
    renderCockpit([atlas, beacon]);

    const table = await screen.findByRole("table");
    expect(table.querySelector("caption")).toHaveTextContent(
      "依 Product 名稱排列，只是瀏覽順序，不是優先順序。",
    );
    const rows = within(table).getAllByRole("rowheader");
    expect(rows.map((row) => row.textContent)).toEqual(["Atlas", "Beacon"]);
    expect(screen.getByRole("button", { name: /^Beacon。/ })).toBeInTheDocument();
  });

  it("keeps an Unknown Product out of every quadrant and says why", async () => {
    renderCockpit([atlas]);

    const table = await screen.findByRole("table");
    const row = within(table).getByRole("row", { name: /Atlas/ });
    expect(row).toHaveTextContent("沒有連結的里程碑");
    expect(row).toHaveTextContent("沒有 KPI 定義");
    expect(row).toHaveTextContent("沒有連結的證據");
    expect(row).toHaveTextContent("資料不足");
  });

  it("draws an Unknown Product outside the plot, in the band that says what is missing", async () => {
    renderCockpit([atlas, beacon]);

    const unknown = await screen.findByRole("button", { name: /^Atlas。/ });
    expect(unknown.closest(".pmc-lens-plot")).toBeNull();
    expect(unknown.closest(".pmc-lens-band-corner")).not.toBeNull();
    const known = screen.getByRole("button", { name: /^Beacon。/ });
    expect(known.closest(".pmc-lens-plot")).not.toBeNull();
  });

  it("puts a point on the side of the divider the host decided", async () => {
    // The host says high with one of three observed; the share alone would
    // put the point below the divider.
    renderCockpit([
      point({
        productId: "product-beacon",
        productName: "Beacon",
        observability: { ...beacon.observability, high: true },
        quadrant: "monitorClosely",
      }),
      point({
        productId: "product-cinder",
        productName: "Cinder",
        observability: { ...beacon.observability, observed: 3, high: false },
      }),
    ]);

    const topOf = (element: HTMLElement) => Number.parseFloat(element.style.top);
    expect(topOf(await screen.findByRole("button", { name: /^Beacon。/ }))).toBeLessThan(50);
    expect(topOf(screen.getByRole("button", { name: /^Cinder。/ }))).toBeGreaterThan(50);
  });

  it("shows ratios as counts, never as percentages", async () => {
    const { container } = renderCockpit([beacon]);
    await screen.findByRole("table");

    expect(container.textContent).toContain("1／3 個 KPI 有觀測");
    expect(container.textContent).toContain("1／2 份已驗證");
    expect(container.textContent).not.toMatch(/%/);
  });

  it("marks what the chosen mode is about without moving anything", async () => {
    renderCockpit([beacon]);
    const bubble = await screen.findByRole("button", { name: /^Beacon。/ });
    const position = bubble.getAttribute("style");

    // Timing mode marks passed dates; Beacon's is only due soon.
    expect(bubble).toHaveAttribute("data-emphasised", "false");

    await userEvent.click(screen.getByRole("button", { name: "成果可觀測" }));
    expect(bubble).toHaveAttribute("data-emphasised", "true");
    expect(screen.getByRole("button", { name: "成果可觀測" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );

    await userEvent.click(screen.getByRole("button", { name: "證據" }));
    expect(bubble).toHaveAttribute("data-emphasised", "true");
    expect(bubble.getAttribute("style")).toBe(position);
  });

  it("opens the measures beside the Lens and moves focus to them", async () => {
    const loadDetail = vi.fn(() => new Promise<ProductDetailDto>(() => undefined));
    renderCockpit([atlas, beacon], loadDetail);

    await userEvent.click(await screen.findByRole("button", { name: /^Beacon。/ }));

    const heading = screen.getByRole("heading", { name: "Beacon 的 Lens 指標" });
    expect(heading).toHaveFocus();
    const inspector = screen.getByRole("complementary", { name: "選取的 Product" });
    expect(inspector).toHaveTextContent("優先處理");
    expect(inspector).toHaveTextContent("快到期，最早");
    expect(inspector).toHaveTextContent("未驗證 1 份");
    // The Evidence link has no version of its own and says so.
    expect(inspector).toHaveTextContent("連結本身沒有版本，以快照版本 9 為準");
    // The O01 inspector for the same Product is asked for.
    expect(loadDetail).toHaveBeenCalledWith("product-beacon");
  });

  it("selects from the table as well as from the chart", async () => {
    renderCockpit([atlas, beacon]);

    await userEvent.click(await screen.findByRole("button", { name: "檢視 Atlas" }));

    expect(screen.getByRole("heading", { name: "Atlas 的 Lens 指標" })).toHaveFocus();
    expect(screen.getByRole("complementary", { name: "選取的 Product" })).toHaveTextContent(
      "資料不足，不歸入任何象限",
    );
    expect(screen.getByRole("button", { name: "已選取 Atlas" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });

  it("names the record that forced a Product's classification", async () => {
    renderCockpit([
      point({
        productId: "product-beacon",
        productName: "Beacon",
        effectiveClassification: "confidential",
        classificationForcedBy: {
          kind: "milestone",
          id: "milestone-7",
          version: 2,
          classification: "confidential",
          role: "milestone",
        },
      }),
    ]);

    await userEvent.click(await screen.findByRole("button", { name: "檢視 Beacon" }));

    const inspector = screen.getByRole("complementary", { name: "選取的 Product" });
    expect(inspector).toHaveTextContent("Confidential");
    expect(inspector).toHaveTextContent("由 Milestone milestone-7 決定");
  });

  it("invites creating a Product when the Ledger has none", async () => {
    renderCockpit([]);

    expect(await screen.findByText(/還沒有任何 Product/)).toBeInTheDocument();
    expect(screen.queryByRole("table")).toBeNull();
  });
});
