import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { EntryOutcomeDto } from "./entryIpc";
import { RecordSheet, type FieldSpec } from "./RecordSheet";

const OUTCOME: EntryOutcomeDto = {
  kind: "product",
  id: "product-1",
  classification: "internal",
  version: 1,
  correlationId: "corr-1",
};

const SIMPLE: readonly FieldSpec[] = [
  { kind: "short", name: "name", label: "名稱", required: true },
  { kind: "long", name: "details", label: "說明", required: true },
  { kind: "classification", name: "classification" },
];

function refusal(errorCode: string, messageKey: string) {
  return Object.assign(new Error("host refused"), {
    errorCode,
    messageKey,
    correlationId: "host-1",
    retryable: false,
    messageParams: [],
    extensions: [],
  });
}

describe("RecordSheet", () => {
  it("submits only once every field passes the domain's own rule, and counts bytes", async () => {
    const submit = vi.fn(() => Promise.resolve(OUTCOME));
    const onDone = vi.fn();
    render(
      <RecordSheet
        title="新增 Product"
        fields={SIMPLE}
        initial={{}}
        submitLabel="建立"
        submit={submit}
        onDone={onDone}
        onClose={() => undefined}
      />,
    );

    const create = screen.getByRole("button", { name: "建立" });
    expect(create).toBeDisabled();
    expect(screen.getByText("尚未選擇分級。")).toBeInTheDocument();

    // Values are set through change events: typing them key by key is what
    // made this test the slowest in the suite and time out under load.
    fireEvent.change(screen.getByLabelText("名稱"), { target: { value: "風險" } });
    // Two three-byte characters: six bytes of the 160 ShortText allows.
    expect(screen.getByText("6 / 160 位元組")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("說明"), { target: { value: "第一行\n第二行" } });
    expect(screen.getByText(/不能含有換行或控制字元/)).toBeInTheDocument();
    expect(create).toBeDisabled();
    fireEvent.change(screen.getByLabelText("說明"), { target: { value: "一行就好" } });
    expect(create).toBeDisabled();

    await userEvent.click(screen.getByLabelText("Internal"));
    expect(screen.queryByText("尚未選擇分級。")).toBeNull();
    expect(create).toBeEnabled();

    await userEvent.click(create);
    await waitFor(() => {
      expect(onDone).toHaveBeenCalledWith(OUTCOME);
    });
    expect(submit).toHaveBeenCalledWith({
      name: "風險",
      details: "一行就好",
      classification: "internal",
    });
  });

  it("refuses a name over the byte limit before any round trip", async () => {
    const submit = vi.fn(() => Promise.resolve(OUTCOME));
    render(
      <RecordSheet
        title="新增 Product"
        fields={SIMPLE}
        initial={{}}
        submitLabel="建立"
        submit={submit}
        onDone={() => undefined}
        onClose={() => undefined}
      />,
    );
    // 54 three-byte characters are 162 bytes: two over ShortText.
    fireEvent.change(screen.getByLabelText("名稱"), { target: { value: "風".repeat(54) } });
    expect(screen.getByText("162 / 160 位元組——超過上限")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("說明"), { target: { value: "ok" } });
    await userEvent.click(screen.getByLabelText("Internal"));
    expect(screen.getByRole("button", { name: "建立" })).toBeDisabled();
    expect(submit).not.toHaveBeenCalled();
  });

  it("closes an untouched sheet at once and asks before discarding a dirty one", async () => {
    const onClose = vi.fn();
    const { unmount } = render(
      <RecordSheet
        title="新增 Product"
        fields={SIMPLE}
        initial={{}}
        submitLabel="建立"
        submit={() => Promise.resolve(OUTCOME)}
        onDone={() => undefined}
        onClose={onClose}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(onClose).toHaveBeenCalledTimes(1);
    unmount();

    const onCloseDirty = vi.fn();
    render(
      <RecordSheet
        title="新增 Product"
        fields={SIMPLE}
        initial={{}}
        submitLabel="建立"
        submit={() => Promise.resolve(OUTCOME)}
        onDone={() => undefined}
        onClose={onCloseDirty}
      />,
    );
    await userEvent.type(screen.getByLabelText("名稱"), "x");
    await userEvent.keyboard("{Escape}");
    expect(onCloseDirty).not.toHaveBeenCalled();
    expect(screen.getByText("要捨棄你輸入的內容嗎？")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "繼續編輯" }));
    expect(screen.queryByText("要捨棄你輸入的內容嗎？")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    await userEvent.click(screen.getByRole("button", { name: "捨棄" }));
    expect(onCloseDirty).toHaveBeenCalledTimes(1);
  });

  it("opens an edit on the record's values and hands a stale version back to the opener", async () => {
    const onStale = vi.fn();
    const submit = vi.fn(() => Promise.reject(refusal("DOMAIN_CONFLICT", "ledger.version_stale")));
    render(
      <RecordSheet
        title="編輯 Product"
        fields={SIMPLE}
        initial={{ name: "Atlas", details: "Old", classification: "confidential" }}
        submitLabel="儲存"
        submit={submit}
        onDone={() => undefined}
        onClose={() => undefined}
        onStale={onStale}
      />,
    );
    expect(screen.getByLabelText("名稱")).toHaveValue("Atlas");
    expect(screen.getByLabelText("Confidential")).toBeChecked();
    // Untouched, so nothing is dirty; still submittable, since every field
    // already passes.
    await userEvent.click(screen.getByRole("button", { name: "儲存" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("已被更動");
    expect(screen.queryByRole("button", { name: "儲存" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "重新載入" }));
    expect(onStale).toHaveBeenCalledTimes(1);
  });

  it("refuses a local time the zone skips, and sends the UTC instant otherwise", async () => {
    const submit = vi.fn(() => Promise.resolve(OUTCOME));
    render(
      <RecordSheet
        title="觀測"
        fields={[
          { kind: "short", name: "value", label: "數值", required: true },
          { kind: "datetime", name: "observedAt", label: "觀測時間", timeZone: "Europe/Berlin" },
          {
            kind: "classification",
            name: "classification",
            inheritLabel: "與 KPI 相同",
          },
        ]}
        initial={{}}
        submitLabel="建立"
        submit={submit}
        onDone={() => undefined}
        onClose={() => undefined}
      />,
    );
    fireEvent.change(screen.getByLabelText("數值"), { target: { value: "7" } });
    await userEvent.click(screen.getByLabelText("與 KPI 相同"));
    fireEvent.change(screen.getByLabelText("觀測時間"), { target: { value: "2026-03-29T02:30" } });
    expect(screen.getByText(/當天不存在，或出現兩次/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "建立" })).toBeDisabled();

    fireEvent.change(screen.getByLabelText("觀測時間"), { target: { value: "2026-03-29T03:30" } });
    expect(screen.getByText("時區：Europe/Berlin。")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "建立" }));
    await waitFor(() => {
      expect(submit).toHaveBeenCalledWith({
        value: "7",
        observedAt: Date.UTC(2026, 2, 29, 1, 30),
        classification: null,
      });
    });
  });

  it("shows a refusal with its correlation id and keeps the sheet open", async () => {
    const submit = vi.fn(() =>
      Promise.reject(refusal("VALIDATION_INVALID_FIELD", "desktop.entry_text_invalid")),
    );
    render(
      <RecordSheet
        title="新增 Product"
        fields={SIMPLE}
        initial={{ name: "A", details: "B", classification: "public" }}
        submitLabel="建立"
        submit={submit}
        onDone={() => undefined}
        onClose={() => undefined}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "建立" }));
    expect(await screen.findByText(/未儲存。/)).toBeInTheDocument();
    expect(screen.getByText("host-1")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "建立" })).toBeEnabled();
  });
});
