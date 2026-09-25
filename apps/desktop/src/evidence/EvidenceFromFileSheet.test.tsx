import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { EvidenceFromFileSheet, type EvidenceFileProduct } from "./EvidenceFromFileSheet";
import type {
  ChosenEvidenceFileDto,
  EvidenceFileActions,
  EvidenceFromFileResultDto,
} from "./evidenceFileIpc";

function chosen(overrides: Partial<ChosenEvidenceFileDto> = {}): ChosenEvidenceFileDto {
  return {
    chosen: true,
    token: "token-1",
    fileName: "launch-review.pdf",
    observedAtMillis: 1_700_000_000_000,
    existing: null,
    sameContent: [],
    ...overrides,
  };
}

function created(evidenceId = "evidence-7"): EvidenceFromFileResultDto {
  return {
    outcome: "created",
    evidence: { evidenceId, version: 1 },
    observedAtMillis: 1_700_000_100_000,
  };
}

function actions(overrides: Partial<EvidenceFileActions> = {}): EvidenceFileActions {
  return {
    chooseEvidenceFile: vi.fn(() => Promise.resolve(chosen())),
    createEvidenceFromFile: vi.fn(() => Promise.resolve(created())),
    discardEvidenceFileChoice: vi.fn(() => Promise.resolve()),
    ...overrides,
  };
}

function product(
  link: EvidenceFileProduct["link"] = vi.fn(() => Promise.resolve({})),
  linked: readonly string[] = [],
): EvidenceFileProduct {
  return { id: "product-1", label: "Atlas", linkedEvidenceIds: new Set(linked), link };
}

function safeError(retryable: boolean, messageKey = "desktop.invalid_argument") {
  return {
    errorCode: retryable ? "PLATFORM_INTERNAL" : "PRODUCT_CHANGED",
    messageKey,
    messageParams: [],
    extensions: [],
    correlationId: "host-9",
    retryable,
  };
}

let sequence = 0;
function ids() {
  sequence += 1;
  return `request-${String(sequence)}`;
}

async function toChosen(fake: EvidenceFileActions, withProduct?: EvidenceFileProduct) {
  const onClose = vi.fn();
  const onFinished = vi.fn();
  render(
    <EvidenceFromFileSheet
      actions={fake}
      product={withProduct}
      onClose={onClose}
      onFinished={onFinished}
      newClientRequestId={ids}
    />,
  );
  await userEvent.click(screen.getByRole("button", { name: "選擇檔案…" }));
  await screen.findByText("檔案：launch-review.pdf");
  return { onClose, onFinished };
}

describe("EvidenceFromFileSheet (item ⑦-3, H1-User)", () => {
  it("shows the file's own name and when it was observed, and creates only after a classification is chosen", async () => {
    const fake = actions();
    const { onFinished } = await toChosen(fake);
    expect(fake.chooseEvidenceFile).toHaveBeenCalledWith("選擇 Product Vault 裡的檔案");
    expect(screen.getByText(/^觀察於 /)).toBeInTheDocument();

    // No default, and never Unclassified (§4.4).
    const radios = screen.getAllByRole<HTMLInputElement>("radio");
    expect(radios.map((radio) => radio.value)).toEqual([
      "public",
      "internal",
      "confidential",
      "restricted",
    ]);
    expect(radios.every((radio) => !radio.checked)).toBe(true);
    const create = screen.getByRole("button", { name: "建立" });
    expect(create).toBeDisabled();

    await userEvent.click(screen.getByRole("radio", { name: "Confidential" }));
    await userEvent.click(create);
    expect(fake.createEvidenceFromFile).toHaveBeenCalledWith(
      "token-1",
      "confidential",
      expect.stringMatching(/^request-/),
    );
    expect(await screen.findByText("已建立 Evidence evidence-7。")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "完成" }));
    expect(onFinished).toHaveBeenCalledTimes(1);
    // The host held the file for a replay until now; closing lets it go.
    expect(fake.discardEvidenceFileChoice).toHaveBeenCalledTimes(1);
  });

  it("stays where it was when the picker is cancelled", async () => {
    const fake = actions({
      chooseEvidenceFile: vi.fn(() =>
        Promise.resolve(chosen({ chosen: false, token: null, fileName: null })),
      ),
    });
    render(
      <EvidenceFromFileSheet
        actions={fake}
        onClose={() => undefined}
        onFinished={() => undefined}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "選擇檔案…" }));
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "選擇檔案…" })).toBeEnabled();
    });
    expect(screen.queryByRole("radio")).toBeNull();
    expect(fake.createEvidenceFromFile).not.toHaveBeenCalled();
  });

  it("forgets the chosen file when the sheet is closed without creating", async () => {
    const fake = actions();
    const { onClose, onFinished } = await toChosen(fake);
    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(fake.discardEvidenceFileChoice).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onFinished).not.toHaveBeenCalled();
  });

  it("warns about the same content under another path and still lets the person create", async () => {
    const fake = actions({
      chooseEvidenceFile: vi.fn(() =>
        Promise.resolve(chosen({ sameContent: [{ evidenceId: "evidence-3", version: 2 }] })),
      ),
    });
    await toChosen(fake);
    expect(screen.getByText("Evidence evidence-3 的內容相同。")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("radio", { name: "Internal" }));
    await userEvent.click(screen.getByRole("button", { name: "建立" }));
    expect(await screen.findByText("已建立 Evidence evidence-7。")).toBeInTheDocument();
  });

  it("shows the new observation when the file changed, and confirms it under the same request", async () => {
    const create = vi
      .fn<EvidenceFileActions["createEvidenceFromFile"]>()
      .mockResolvedValueOnce({
        outcome: "file_changed",
        evidence: null,
        observedAtMillis: 1_700_000_500_000,
      })
      .mockResolvedValueOnce(created());
    const fake = actions({ createEvidenceFromFile: create });
    await toChosen(fake);
    await userEvent.click(screen.getByRole("radio", { name: "Restricted" }));
    await userEvent.click(screen.getByRole("button", { name: "建立" }));

    expect(await screen.findByText("選擇之後，這個檔案已經變更。")).toBeInTheDocument();
    // Nothing was created: the person confirms the new observation first.
    expect(screen.queryByText(/已建立/)).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "建立" }));
    expect(await screen.findByText("已建立 Evidence evidence-7。")).toBeInTheDocument();
    expect(create).toHaveBeenCalledTimes(2);
    expect(create.mock.calls[1]).toEqual(create.mock.calls[0]);
  });

  it("retries a create whose reply was lost as the same create", async () => {
    const create = vi
      .fn<EvidenceFileActions["createEvidenceFromFile"]>()
      .mockRejectedValueOnce(new Error("transport lost"))
      .mockResolvedValueOnce(created());
    const fake = actions({ createEvidenceFromFile: create });
    await toChosen(fake);
    await userEvent.click(screen.getByRole("radio", { name: "Public" }));
    await userEvent.click(screen.getByRole("button", { name: "建立" }));
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "建立" })).toBeEnabled();
    });
    await userEvent.click(screen.getByRole("button", { name: "建立" }));
    expect(await screen.findByText("已建立 Evidence evidence-7。")).toBeInTheDocument();
    // Same token, same request id: the host's reservation replays it.
    expect(create.mock.calls[1]).toEqual(create.mock.calls[0]);
  });

  it("offers nothing to create when a reference already names the file (S08)", async () => {
    const fake = actions({
      chooseEvidenceFile: vi.fn(() =>
        Promise.resolve(chosen({ existing: { evidenceId: "evidence-2", version: 4 } })),
      ),
    });
    await toChosen(fake);
    expect(
      screen.getByText("Evidence evidence-2 已經參照這個檔案，所以不會再建立新的。"),
    ).toBeInTheDocument();
    expect(screen.queryByRole("radio")).toBeNull();
    expect(screen.queryByRole("button", { name: "建立" })).toBeNull();
    expect(screen.queryByRole("button", { name: "把它連結到這個 Product" })).toBeNull();
  });
});

describe("EvidenceFromFileSheet from O01 (create and link)", () => {
  it("creates, then links the new reference at its version under a request id of its own", async () => {
    const link = vi.fn(() => Promise.resolve({}));
    const fake = actions();
    const { onFinished } = await toChosen(fake, product(link));
    await userEvent.click(screen.getByRole("radio", { name: "Public" }));
    await userEvent.click(screen.getByRole("button", { name: "建立並連結到這個 Product" }));

    expect(
      await screen.findByText("已建立 Evidence evidence-7，並連結到 Atlas。"),
    ).toBeInTheDocument();
    const createId = vi.mocked(fake.createEvidenceFromFile).mock.calls[0]?.[2];
    expect(link).toHaveBeenCalledWith("evidence-7", 1, `${String(createId)}-link`);
    await userEvent.click(screen.getByRole("button", { name: "完成" }));
    expect(onFinished).toHaveBeenCalledTimes(1);
  });

  it("says so when the link fails after the create, and the retry resumes the link only", async () => {
    const link = vi
      .fn<EvidenceFileProduct["link"]>()
      .mockRejectedValueOnce(safeError(true))
      .mockResolvedValueOnce({});
    const fake = actions();
    await toChosen(fake, product(link));
    await userEvent.click(screen.getByRole("radio", { name: "Internal" }));
    await userEvent.click(screen.getByRole("button", { name: "建立並連結到這個 Product" }));

    expect(await screen.findByText("已建立 Evidence evidence-7；尚未連結。")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "現在連結" }));
    expect(
      await screen.findByText("已建立 Evidence evidence-7，並連結到 Atlas。"),
    ).toBeInTheDocument();
    expect(fake.createEvidenceFromFile).toHaveBeenCalledTimes(1);
    expect(link).toHaveBeenCalledTimes(2);
    expect(link.mock.calls[1]).toEqual(link.mock.calls[0]);
  });

  it("offers no retry when trying the link again cannot succeed", async () => {
    const link = vi
      .fn<EvidenceFileProduct["link"]>()
      .mockRejectedValue(safeError(false, "product.stale_version"));
    const fake = actions();
    await toChosen(fake, product(link));
    await userEvent.click(screen.getByRole("radio", { name: "Internal" }));
    await userEvent.click(screen.getByRole("button", { name: "建立並連結到這個 Product" }));
    expect(await screen.findByText("已建立 Evidence evidence-7；尚未連結。")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "現在連結" })).toBeNull();
  });

  it("offers to link the existing reference instead of creating another", async () => {
    const link = vi.fn(() => Promise.resolve({}));
    const fake = actions({
      chooseEvidenceFile: vi.fn(() =>
        Promise.resolve(chosen({ existing: { evidenceId: "evidence-2", version: 4 } })),
      ),
    });
    await toChosen(fake, product(link));
    await userEvent.click(screen.getByRole("button", { name: "把它連結到這個 Product" }));
    expect(await screen.findByText("已把 Evidence evidence-2 連結到 Atlas。")).toBeInTheDocument();
    expect(link).toHaveBeenCalledWith("evidence-2", 4, expect.stringMatching(/-link$/));
    expect(fake.createEvidenceFromFile).not.toHaveBeenCalled();
  });

  it("says the existing reference is already linked here, and offers no link", async () => {
    const fake = actions({
      chooseEvidenceFile: vi.fn(() =>
        Promise.resolve(chosen({ existing: { evidenceId: "evidence-2", version: 4 } })),
      ),
    });
    await toChosen(fake, product(undefined, ["evidence-2"]));
    expect(
      screen.getByText("Evidence evidence-2 已經參照這個檔案，也已經連結到這個 Product。"),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "把它連結到這個 Product" })).toBeNull();
  });
});
