import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type {
  EvidenceReferencesDto,
  ExceptionDto,
  ExecutiveCockpitDto,
  LedgerStatusDto,
  VaultStatusDto,
} from "./cockpitContract";
import { LocaleRoot } from "../i18n/LocaleRoot";
import { Reviews } from "./Reviews";
import { Settings } from "./Settings";
import { Vault } from "./Vault";

const NOW = 1_700_000_000_000;

function exception(overrides: Partial<ExceptionDto> = {}): ExceptionDto {
  return {
    kind: "action_request",
    id: "request-1",
    label: "Approve the pricing experiment",
    reason: "action_request_response_overdue",
    explanation: "the response deadline has passed",
    tier: "breachedCommitment",
    tierWhy: "a commitment has already been missed",
    relevantAtMillis: NOW - 1,
    rankRationale: "placed by a breached commitment",
    classification: "internal",
    freshness: "fresh",
    degraded: false,
    ...overrides,
  };
}

function cockpit(overrides: Partial<ExecutiveCockpitDto> = {}): ExecutiveCockpitDto {
  return {
    state: "success",
    asOfMillis: NOW,
    ledgerRevision: 9,
    periodComparable: false,
    periodNote: "no review period has been approved yet, so there is nothing to compare against",
    pulse: {
      milestones: { count: 0, definition: "d", owner: "delivery" },
      commitments: { count: 0, definition: "d", owner: "action_management" },
      kpis: { count: 0, definition: "d", owner: "kpi" },
    },
    products: [],
    exceptions: [
      exception(),
      exception({ reason: "action_request_missing_intended_owner", tier: "noAccountableOwner" }),
      exception({
        kind: "decision_request",
        id: "decision-1",
        label: "Decide the renewal term",
        reason: "decision_request_missing_decision_owner",
        tier: "noAccountableOwner",
      }),
    ],
    leaderConclusion: "",
    leaderIntervention: null,
    lens: { asOfMillis: NOW, ledgerRevision: 9, dueSoonWindowMillis: 1, points: [] },
    ...overrides,
  };
}

describe("Reviews & Reports", () => {
  it("lists each flagged record once, in the Cockpit's order, with its first reason", async () => {
    render(<Reviews load={() => Promise.resolve(cockpit())} onOpenWorkQueue={vi.fn()} />);

    const list = await screen.findByRole("list");
    const items = list.querySelectorAll("li");
    expect(items).toHaveLength(2);
    expect(items[0]).toHaveTextContent("Approve the pricing experiment");
    expect(items[0]).toHaveTextContent("回覆期限已經過了");
    expect(items[1]).toHaveTextContent("Decide the renewal term");
    expect(screen.getByText(/共 2 件/)).toBeInTheDocument();
  });

  it("says plainly that there is no review period and no report workflow yet", async () => {
    render(<Reviews load={() => Promise.resolve(cockpit())} onOpenWorkQueue={vi.fn()} />);

    expect(
      await screen.findByText(/沒有可用的檢視期間：還沒有核准任何檢視期間/),
    ).toBeInTheDocument();
    expect(screen.getByText(/還沒有建立 Fact Pack 與核准報告的功能/)).toBeInTheDocument();
  });

  it("sends the person to the Work Queue to act", async () => {
    const open = vi.fn();
    render(<Reviews load={() => Promise.resolve(cockpit())} onOpenWorkQueue={open} />);

    await userEvent.click(await screen.findByRole("button", { name: "到 Work Queue 處置" }));
    expect(open).toHaveBeenCalledOnce();
  });

  it("says nothing is flagged rather than offering an empty list", async () => {
    render(
      <Reviews
        load={() => Promise.resolve(cockpit({ exceptions: [] }))}
        onOpenWorkQueue={vi.fn()}
      />,
    );

    expect(await screen.findByText("Work Queue 目前沒有標記任何事。")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "到 Work Queue 處置" })).toBeNull();
  });

  it("shows nothing but a re-read when the two Ledger reads disagree", async () => {
    render(
      <Reviews
        load={() => Promise.resolve(cockpit({ state: "outOfSync", exceptions: [] }))}
        onOpenWorkQueue={vi.fn()}
      />,
    );

    expect(await screen.findByText(/版本不一致/)).toBeInTheDocument();
    expect(screen.queryByRole("list")).toBeNull();
  });
});

const available: VaultStatusDto = {
  configured: true,
  available: true,
  reason: null,
  folderName: null,
  correlationId: "c-1",
};

function references(): EvidenceReferencesDto {
  return {
    ledgerRevision: 9,
    correlationId: "c-2",
    evidenceReferences: [
      {
        id: "evidence-2",
        role: null,
        verification: { kind: "observed_unpinned", atMillis: NOW, integrityDigest: null },
        pinned: false,
        classification: "restricted",
        version: 2,
      },
      {
        id: "evidence-1",
        role: null,
        verification: { kind: "verified", atMillis: NOW, integrityDigest: "a".repeat(64) },
        pinned: true,
        classification: "internal",
        version: 1,
      },
    ],
  };
}

describe("Product Vault", () => {
  it("opens Evidence from a file, create only, and reads the page again after", async () => {
    const loadReferences = vi.fn(() => Promise.resolve(references()));
    const evidenceFile = {
      chooseEvidenceFile: vi.fn(() =>
        Promise.resolve({
          chosen: true,
          token: "token-1",
          fileName: "notes.md",
          observedAtMillis: 1_700_000_000_000,
          existing: null,
          sameContent: [],
        }),
      ),
      createEvidenceFromFile: vi.fn(() =>
        Promise.resolve({
          outcome: "created" as const,
          evidence: { evidenceId: "evidence-9", version: 1 },
          observedAtMillis: 1_700_000_000_000,
        }),
      ),
      discardEvidenceFileChoice: vi.fn(() => Promise.resolve()),
    };
    render(
      <Vault
        loadVaultStatus={() => Promise.resolve(available)}
        loadEvidenceReferences={loadReferences}
        evidenceFile={evidenceFile}
      />,
    );
    await userEvent.click(await screen.findByRole("button", { name: "從檔案新增 Evidence…" }));
    await userEvent.click(await screen.findByRole("button", { name: "選擇檔案…" }));
    await userEvent.click(await screen.findByRole("radio", { name: "Restricted" }));
    // S08 creates only: no Product to link (§4.6).
    await userEvent.click(screen.getByRole("button", { name: "建立" }));
    expect(await screen.findByText("已建立 Evidence evidence-9。")).toBeInTheDocument();
    const reads = loadReferences.mock.calls.length;
    await userEvent.click(screen.getByRole("button", { name: "完成" }));
    await waitFor(() => {
      expect(loadReferences.mock.calls.length).toBeGreaterThan(reads);
    });
  });

  it("lists every Evidence reference by identifier with its state in words", async () => {
    render(
      <Vault
        loadVaultStatus={() => Promise.resolve(available)}
        loadEvidenceReferences={() => Promise.resolve(references())}
      />,
    );

    const headers = await screen.findAllByRole("rowheader");
    expect(headers.map((header) => header.textContent)).toEqual(["evidence-1", "evidence-2"]);
    const second = screen.getByRole("row", { name: /evidence-2/ });
    expect(second).toHaveTextContent("讀得到，但沒有釘選指紋");
    expect(second).toHaveTextContent("沒有釘選");
    expect(second).toHaveTextContent("Restricted");
  });

  it("says why the Vault cannot be used and what still works", async () => {
    render(
      <Vault
        loadVaultStatus={() =>
          Promise.resolve({
            configured: false,
            available: false,
            reason: "notConfigured",
            folderName: null,
            correlationId: "c-3",
          })
        }
        loadEvidenceReferences={() => Promise.resolve(references())}
      />,
    );

    const status = await screen.findByText(/Vault 無法使用/);
    expect(status.closest("p")).toHaveTextContent("這個工作區沒有設定 Product Vault");
    expect(status.closest("p")).toHaveTextContent("連結 Evidence 不受影響");
  });

  it("shows no Vault path anywhere", async () => {
    const { container } = render(
      <Vault
        loadVaultStatus={() => Promise.resolve(available)}
        loadEvidenceReferences={() => Promise.resolve(references())}
      />,
    );
    await screen.findByRole("table");

    expect(container.textContent).not.toMatch(/[A-Za-z]:\\|\/Users\//);
  });

  it("reports a failed read without showing stale data", async () => {
    render(
      <Vault
        loadVaultStatus={() => Promise.resolve(available)}
        loadEvidenceReferences={() => Promise.reject<EvidenceReferencesDto>(new Error("down"))}
      />,
    );

    expect(await screen.findByRole("alert")).toHaveTextContent("沒有顯示任何舊資料");
    expect(screen.queryByRole("table")).toBeNull();
  });
});

describe("Settings", () => {
  const ledger: LedgerStatusDto = { schemaVersion: 46, revision: 166 };

  it("states the Ledger and Vault this workspace opened, without paths", async () => {
    const { container } = render(
      <Settings
        loadLedgerStatus={() => Promise.resolve(ledger)}
        loadVaultStatus={() => Promise.resolve(available)}
      />,
    );

    expect(await screen.findByText(/資料結構版本 46，目前 Ledger 版本 166/)).toBeInTheDocument();
    expect(screen.getByText("可以使用")).toBeInTheDocument();
    expect(container.textContent).not.toMatch(/[A-Za-z]:\\/);
  });

  it("offers to choose the Vault folder only where the person chooses it, and says what is set", async () => {
    // Item ⑦: Live's Vault is the person's to choose. Without the actions
    // (Training, whose Vault is its own) there is nothing to offer.
    const { unmount } = render(
      <Settings
        loadLedgerStatus={() => Promise.resolve(ledger)}
        loadVaultStatus={() => Promise.resolve(available)}
      />,
    );
    await screen.findByText(/166/);
    expect(screen.queryByRole("button", { name: "更換 Vault 資料夾…" })).toBeNull();
    expect(screen.queryByRole("button", { name: "選擇 Vault 資料夾…" })).toBeNull();
    unmount();

    const unset: VaultStatusDto = {
      configured: false,
      available: false,
      reason: "notConfigured",
      folderName: null,
      correlationId: "c-9",
    };
    render(
      <Settings
        loadLedgerStatus={() => Promise.resolve(ledger)}
        loadVaultStatus={() => Promise.resolve(unset)}
        vaultRoot={{
          chooseVaultFolder: vi.fn(() =>
            Promise.resolve({ chosen: false, token: null, folderName: null }),
          ),
          prepareVaultRootChange: vi.fn(),
          rejectVaultRootChange: vi.fn(),
          approveVaultRootChange: vi.fn(),
        }}
      />,
    );
    expect(await screen.findByText("尚未設定")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "選擇 Vault 資料夾…" }));
    expect(await screen.findByRole("dialog", { name: "Product Vault 資料夾" })).toBeInTheDocument();
  });

  it("names a set Vault folder even when it cannot be reached", async () => {
    const unplugged: VaultStatusDto = {
      configured: true,
      available: false,
      reason: "invalidRoot",
      folderName: "Team Evidence",
      correlationId: "c-10",
    };
    render(
      <Settings
        loadLedgerStatus={() => Promise.resolve(ledger)}
        loadVaultStatus={() => Promise.resolve(unplugged)}
      />,
    );
    expect(await screen.findByText(/Team Evidence/)).toBeInTheDocument();
    expect(screen.queryByText("尚未設定")).toBeNull();
  });

  it("applies and remembers the chosen text scale for the whole app", async () => {
    render(
      <Settings
        loadLedgerStatus={() => Promise.resolve(ledger)}
        loadVaultStatus={() => Promise.resolve(available)}
      />,
    );

    await userEvent.click(await screen.findByRole("button", { name: "120%" }));

    await waitFor(() => {
      expect(document.documentElement.style.getPropertyValue("--pmc-app-scale")).toBe("1.2");
    });
    expect(window.localStorage.getItem("pmc-app-text-scale")).toBe("120");
    expect(screen.getByRole("button", { name: "120%" })).toHaveAttribute("aria-pressed", "true");
  });

  function languageSettings(save: (preference: string) => Promise<unknown>) {
    return render(
      <LocaleRoot initial="zh-TW" systemTags={["ko-KR"]} save={save}>
        <Settings
          loadLedgerStatus={() => Promise.resolve(ledger)}
          loadVaultStatus={() => Promise.resolve(available)}
        />
      </LocaleRoot>,
    );
  }

  it("offers the system's language and all six, each named in itself", async () => {
    languageSettings(() => Promise.resolve());
    const select = await screen.findByRole("combobox", { name: "語言" });

    expect(select).toHaveValue("zh-TW");
    expect(
      Array.from((select as HTMLSelectElement).options).map((option) => option.textContent),
    ).toEqual([
      "跟隨系統（한국어）",
      "English",
      "繁體中文",
      "简体中文",
      "日本語",
      "한국어",
      "Español",
    ]);
  });

  it("stores a new language, then switches the whole screen and <html lang> to it", async () => {
    const save = vi.fn(() => Promise.resolve());
    languageSettings(save);

    await userEvent.selectOptions(await screen.findByRole("combobox", { name: "語言" }), "es");

    expect(save).toHaveBeenCalledWith("es");
    expect(await screen.findByRole("combobox", { name: "Idioma" })).toHaveValue("es");
    expect(document.documentElement.lang).toBe("es");
    expect(screen.getByText("Tamaño del texto")).toBeInTheDocument();
  });

  it("switches only once the host has stored it, and takes one change at a time", async () => {
    let finish: () => void = () => undefined;
    const save = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    );
    languageSettings(save);
    const select = await screen.findByRole("combobox", { name: "語言" });

    await userEvent.selectOptions(select, "es");

    expect(select).toHaveValue("zh-TW");
    expect(select).toBeDisabled();
    expect(document.documentElement.lang).toBe("zh-TW");
    expect(screen.getByText("文字縮放")).toBeInTheDocument();

    finish();

    expect(await screen.findByRole("combobox", { name: "Idioma" })).toHaveValue("es");
    expect(screen.getByRole("combobox", { name: "Idioma" })).toBeEnabled();
    expect(document.documentElement.lang).toBe("es");
    expect(save).toHaveBeenCalledTimes(1);
  });

  it("follows the system when asked to", async () => {
    languageSettings(() => Promise.resolve());

    await userEvent.selectOptions(await screen.findByRole("combobox", { name: "語言" }), "und");

    expect(await screen.findByRole("combobox", { name: "언어" })).toHaveValue("und");
    expect(document.documentElement.lang).toBe("ko");
  });

  it("keeps the language it had when the new one could not be stored, and says so", async () => {
    languageSettings(() =>
      // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- the host rejects with its safe envelope, not an Error.
      Promise.reject({
        errorCode: "PLATFORM_INTERNAL",
        messageKey: "desktop.settings_unavailable",
        messageParams: [],
        correlationId: "host-9",
        retryable: false,
        extensions: [],
      }),
    );

    await userEvent.selectOptions(await screen.findByRole("combobox", { name: "語言" }), "ja");

    expect(
      await screen.findByText(/語言沒有儲存成功。目前無法讀取或儲存顯示設定。/),
    ).toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: "語言" })).toHaveValue("zh-TW");
    expect(document.documentElement.lang).toBe("zh-TW");
  });

  it("shows no language setting when rendered on its own", async () => {
    render(
      <Settings
        loadLedgerStatus={() => Promise.resolve(ledger)}
        loadVaultStatus={() => Promise.resolve(available)}
      />,
    );
    await screen.findByText("可以使用");

    expect(screen.queryByRole("combobox")).toBeNull();
  });
});
