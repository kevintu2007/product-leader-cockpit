import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { PeopleDirectoryDto } from "./cockpitContract";
import { People } from "./People";
import { entryTestActions } from "../entry/entryTestActions";

function directory(overrides: Partial<PeopleDirectoryDto> = {}): PeopleDirectoryDto {
  return {
    state: "success",
    asOfMillis: 1_700_000_000_000,
    ledgerRevision: 7,
    people: [
      {
        id: "stakeholder-a",
        displayName: "Synthetic Person A",
        kind: "person",
        classification: "restricted",
        revision: 1,
        owner: "stakeholders",
        responsibilityCount: 1,
        dependencyCount: 0,
        outstandingRequestCount: 1,
        relationships: [
          {
            subjectId: "milestone-1",
            subjectLabel: "Launch pricing page",
            purpose: "responsibility",
          },
        ],
        outstandingRequests: [{ id: "request-1", label: "Approve the pricing experiment" }],
      },
      {
        id: "stakeholder-b",
        displayName: "Synthetic Vendor B",
        kind: "organization",
        classification: "internal",
        revision: 2,
        owner: "stakeholders",
        responsibilityCount: 0,
        dependencyCount: 2,
        outstandingRequestCount: 0,
        relationships: [
          { subjectId: "project-1", subjectLabel: "Hosting renewal", purpose: "dependency" },
          { subjectId: "project-2", subjectLabel: "Data export", purpose: "dependency" },
        ],
        outstandingRequests: [],
      },
    ],
    offset: 0,
    limit: 25,
    total: 2,
    hasMore: false,
    ...overrides,
  };
}

describe("People", () => {
  it("shows the effective classification rather than the record's own label", async () => {
    // `stakeholder-a` is an Internal person responsible for a Restricted
    // Milestone. Composition folds classification upward, and the route must
    // show the folded value: putting a person together with what they are
    // accountable for is itself a disclosure.
    render(<People load={() => Promise.resolve(directory())} />);

    const row = within(await screen.findByRole("row", { name: /Synthetic Person A/ }));
    expect(row.getByText("Restricted")).toBeInTheDocument();
  });

  it("names what each person is responsible for and depends on, in separate columns", async () => {
    // One combined "involvement" list would say neither.
    render(<People load={() => Promise.resolve(directory())} />);

    const headers = (await screen.findAllByRole("columnheader")).map(
      (header) => header.textContent,
    );
    const responsibility = headers.indexOf("負責");
    const dependency = headers.indexOf("依賴");
    expect(responsibility).toBeGreaterThan(-1);
    expect(dependency).toBeGreaterThan(-1);

    const vendor = within(screen.getByRole("row", { name: /Synthetic Vendor B/ })).getAllByRole(
      "cell",
    );
    // Cells follow the row header, so column n is cell n - 1.
    expect(vendor[responsibility - 1]).toHaveTextContent("沒有");
    expect(vendor[dependency - 1]).toHaveTextContent("Hosting renewal");
    expect(vendor[dependency - 1]).toHaveTextContent("Data export");
  });

  it("lists the requests still waiting on a person by title, as requests", async () => {
    render(<People load={() => Promise.resolve(directory())} />);

    const row = await screen.findByRole("row", { name: /Synthetic Person A/ });
    expect(row).toHaveTextContent("Action Request");
    expect(row).toHaveTextContent("Approve the pricing experiment");
    expect(row).not.toHaveTextContent("request-1");
  });

  it("says whether an entry is a person or an organization", async () => {
    render(<People load={() => Promise.resolve(directory())} />);

    expect(await screen.findByRole("row", { name: /Synthetic Vendor B/ })).toHaveTextContent(
      "組織",
    );
  });

  it("states the page position rather than leaving it to be inferred", async () => {
    render(
      <People load={() => Promise.resolve(directory({ offset: 25, total: 60, hasMore: true }))} />,
    );

    expect(await screen.findByText(/顯示第 26 到 27 位，共 60 位/)).toBeInTheDocument();
  });

  it("uses names as row headers so a row stays identifiable cell by cell", async () => {
    render(<People load={() => Promise.resolve(directory())} />);

    const rowHeaders = await screen.findAllByRole("rowheader");
    expect(rowHeaders.map((header) => header.textContent)).toEqual([
      "Synthetic Person A",
      "Synthetic Vendor B",
    ]);
  });

  it("renders no progress percentage anywhere", async () => {
    const { container } = render(<People load={() => Promise.resolve(directory())} />);
    await screen.findByRole("table");

    expect(container.textContent).not.toMatch(/%/);
    expect(container.querySelector("progress")).toBeNull();
  });

  it("says the directory is empty rather than rendering an empty table", async () => {
    render(<People load={() => Promise.resolve(directory({ people: [], total: 0 }))} />);

    expect(await screen.findByText(/還沒有任何 Stakeholder/)).toBeInTheDocument();
    expect(screen.queryByRole("table")).toBeNull();
  });

  it("reports a failed query without showing stale data", async () => {
    render(<People load={() => Promise.reject<PeopleDirectoryDto>(new Error("unavailable"))} />);

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("沒有顯示任何舊資料");
    expect(screen.queryByRole("table")).toBeNull();
  });

  it("pages forward and backward by keyboard", async () => {
    const load = vi.fn((offset: number) =>
      Promise.resolve(
        directory({
          offset,
          total: 60,
          hasMore: offset + 25 < 60,
        }),
      ),
    );
    render(<People load={load} />);
    await screen.findByRole("table");

    await userEvent.click(screen.getByRole("button", { name: "下一頁" }));

    await waitFor(() => {
      expect(load).toHaveBeenCalledWith(25, 25);
    });
  });

  it("disables paging at the ends rather than letting it run past them", async () => {
    render(<People load={() => Promise.resolve(directory())} />);
    await screen.findByRole("table");

    expect(screen.getByRole("button", { name: "上一頁" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "下一頁" })).toBeDisabled();
  });
});

describe("People record entry (slice 6D)", () => {
  it("offers nothing to enter without the entry actions", async () => {
    render(<People load={() => Promise.resolve(directory())} />);
    await screen.findByRole("table");
    expect(screen.queryByRole("button", { name: "新增 Stakeholder…" })).toBeNull();
    expect(screen.queryByRole("button", { name: "編輯…" })).toBeNull();
  });

  it("creates a Stakeholder with a chosen kind under one request id", async () => {
    const actions = entryTestActions();
    render(<People load={() => Promise.resolve(directory())} entryActions={actions} />);
    await screen.findByRole("table");
    await userEvent.click(screen.getByRole("button", { name: "新增 Stakeholder…" }));
    const dialog = await screen.findByRole("dialog", { name: "新增 Stakeholder" });
    await userEvent.type(within(dialog).getByLabelText("名稱"), "王小明");
    expect(within(dialog).getByRole("button", { name: "建立" })).toBeDisabled();
    await userEvent.selectOptions(within(dialog).getByLabelText("類型"), "person");
    await userEvent.click(within(dialog).getByLabelText("Internal"));
    await userEvent.click(within(dialog).getByRole("button", { name: "建立" }));
    await waitFor(() => {
      expect(actions.createStakeholder).toHaveBeenCalledWith(
        { name: "王小明", classification: "internal" },
        "person",
        expect.stringMatching(/^entry-/),
      );
    });
    expect(await screen.findByRole("status")).toHaveTextContent(
      "已建立 Stakeholder stakeholder-new。",
    );
  });

  it("edits a Stakeholder at the version read, without its kind", async () => {
    const actions = entryTestActions({}, [
      {
        kind: "stakeholder",
        id: "stakeholder-b",
        name: "Synthetic Vendor B",
        stakeholderKind: "organization",
        classification: "internal",
        version: 2,
      },
    ]);
    render(<People load={() => Promise.resolve(directory())} entryActions={actions} />);
    await screen.findByRole("table");
    const row = screen.getByRole("row", { name: /Synthetic Vendor B/ });
    await userEvent.click(within(row).getByRole("button", { name: "編輯…" }));
    const dialog = await screen.findByRole("dialog", { name: "編輯 Stakeholder" });
    expect(within(dialog).queryByLabelText("類型")).toBeNull();
    await userEvent.clear(within(dialog).getByLabelText("名稱"));
    await userEvent.type(within(dialog).getByLabelText("名稱"), "Vendor B, renamed");
    await userEvent.click(within(dialog).getByRole("button", { name: "儲存" }));
    await waitFor(() => {
      expect(actions.updateStakeholder).toHaveBeenCalledWith(
        "stakeholder-b",
        2,
        { name: "Vendor B, renamed", classification: "internal" },
        expect.stringMatching(/^entry-/),
      );
    });
  });

  it("relates a Stakeholder to a subject of a chosen kind at both versions", async () => {
    const actions = entryTestActions({}, [
      {
        kind: "stakeholder",
        id: "stakeholder-a",
        name: "Synthetic Person A",
        stakeholderKind: "person",
        classification: "confidential",
        version: 1,
      },
      {
        kind: "project",
        id: "project-3",
        name: "Hosting migration",
        startAtMillis: 1,
        endAtMillis: 2,
        classification: "internal",
        version: 5,
      },
    ]);
    render(<People load={() => Promise.resolve(directory())} entryActions={actions} />);
    await screen.findByRole("table");
    const row = screen.getByRole("row", { name: /Synthetic Person A/ });
    await userEvent.click(within(row).getByRole("button", { name: "建立與對象的關係…" }));
    const dialog = await screen.findByRole("dialog", {
      name: "建立 Synthetic Person A 與對象的關係",
    });
    const link = within(dialog).getByRole("button", { name: "連結" });
    expect(link).toBeDisabled();
    await userEvent.click(within(dialog).getByLabelText("依賴"));
    await userEvent.selectOptions(within(dialog).getByLabelText("對象類型"), "project");
    await waitFor(() => {
      expect(actions.listEntryRecords).toHaveBeenCalledWith("project");
    });
    await within(dialog).findByRole("option", { name: "Hosting migration（Internal，版本 5）" });
    await userEvent.selectOptions(within(dialog).getByLabelText("要連結的紀錄"), "project-3");
    // The Stakeholder's own classification (Confidential, not the folded
    // Restricted the directory shows) combined with the subject's.
    expect(within(dialog).getByText("這個連結會以 Confidential 分級記錄。")).toBeInTheDocument();
    await userEvent.click(link);
    await waitFor(() => {
      expect(actions.linkStakeholderSubject).toHaveBeenCalledWith(
        "stakeholder-a",
        1,
        "project",
        "project-3",
        5,
        "dependency",
        expect.stringMatching(/^entry-/),
      );
    });
    expect(await screen.findByRole("status")).toHaveTextContent("已連結 relationship-6。");
  });
});
