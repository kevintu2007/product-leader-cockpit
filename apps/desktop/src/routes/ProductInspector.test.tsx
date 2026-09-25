import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { ProductDetailDto } from "./cockpitContract";
import type {
  EvidenceReferencesDto,
  EvidenceWriteOutcomeDto,
  VaultStatusDto,
} from "./cockpitContract";
import { ProductInspector, type EvidenceActions } from "./ProductInspector";
import { entryTestActions } from "../entry/entryTestActions";
import { fireEvent, waitFor } from "@testing-library/react";

function detail(overrides: Partial<ProductDetailDto> = {}): ProductDetailDto {
  return {
    state: "success",
    asOfMillis: 1_700_000_000_000,
    ledgerRevision: 7,
    product: {
      id: "product-1",
      label: "Synthetic Product",
      classification: "restricted",
      revision: 2,
      owner: "portfolio",
      asOfMillis: 1_700_000_000_000,
    },
    classificationForcedBy: {
      forcedByKind: "stakeholder",
      forcedById: "stakeholder-a",
      classification: "restricted",
    },
    structure: [
      {
        kind: "project",
        id: "project-1",
        label: "Synthetic Project",
        classification: "internal",
        revision: 1,
        via: null,
      },
      {
        kind: "initiative",
        id: "initiative-1",
        label: "Synthetic Initiative",
        classification: "internal",
        revision: 1,
        via: "project:project-1",
      },
    ],
    evidence: [
      {
        id: "evidence-1",
        role: null,
        verification: "unverified",
        verifiedAtMillis: null,
        pinned: false,
        classification: "restricted",
        classificationAtLink: "internal",
        linkedAtMillis: 6_000,
        revision: 3,
      },
    ],
    people: [
      {
        id: "stakeholder-a",
        displayName: "Synthetic Person",
        purpose: "responsibility",
        classification: "restricted",
        revision: 1,
        otherProductsAccountableFor: 1,
        carried: [
          {
            kind: "action",
            id: "action-1",
            label: "Action action-1",
            stateLabel: "Open",
            classification: "internal",
            revision: 8,
            asOfMillis: 1_699_500_000_000,
            lifecycleLegalIntents: ["start_action", "prepare_cancel_action"],
            attention: [
              {
                reason: "action_overdue",
                explanation: "the action passed its due date",
                tier: "breachedCommitment",
                tierWhy: "a commitment has already been missed",
                relevantAtMillis: 1_000,
                rankRationale: "placed by a breached commitment",
                classification: "internal",
                freshness: "fresh",
                degraded: false,
              },
            ],
          },
        ],
      },
    ],
    healthReasons: [
      {
        text: "Evidence evidence-1 is linked, but it has not been verified",
        reasonCode: "unverified",
        subjectKind: "evidence",
        subjectLabel: "evidence-1",
        owner: "evidence",
        sourceRecordId: "evidence-1",
        sourceField: "verification",
        sourceRevision: 3,
        asOfMillis: 1_699_000_000_000,
      },
    ],
    impact: "unassessed",
    ...overrides,
  };
}

function inspector(dto: ProductDetailDto = detail()) {
  return <ProductInspector productId="product-1" load={() => Promise.resolve(dto)} />;
}

describe("ProductInspector", () => {
  it("names the Product and says which child forced its classification", async () => {
    render(inspector());

    expect(await screen.findByRole("heading", { name: "Synthetic Product" })).toBeInTheDocument();
    expect(
      screen.getByText(/因為它顯示的 Stakeholder stakeholder-a 是這個分級/),
    ).toBeInTheDocument();
  });

  it("reports 影響 as unassessed rather than computing one", async () => {
    render(inspector());
    expect(await screen.findByText("尚未有人評估")).toBeInTheDocument();
  });

  it("words each condition and attributes it to the record that raised it", async () => {
    render(inspector());
    const condition = (
      await screen.findByText(/Evidence evidence-1 已連結；驗證狀態：未驗證/)
    ).closest("li");
    expect(condition).toHaveTextContent("來源：Evidence 管理 evidence-1，版本 3");
    expect(condition).not.toHaveTextContent("has not been verified");
  });

  it("words a condition raised on carried work by its record and reason", async () => {
    render(
      inspector(
        detail({
          healthReasons: [
            {
              text: "the due date has passed",
              reasonCode: "action_overdue",
              subjectKind: "action",
              subjectLabel: "Ship the pricing page",
              owner: "action_management",
              sourceRecordId: "action-1",
              sourceField: "attention",
              sourceRevision: 4,
              asOfMillis: 1_699_000_000_000,
            },
          ],
        }),
      ),
    );

    const condition = (await screen.findByText(/Ship the pricing page/)).closest("li");
    expect(condition).toHaveTextContent("Action「Ship the pricing page」");
    expect(condition).not.toHaveTextContent("the due date has passed");
  });

  it("shows the host's own sentence when it could not name the reason", async () => {
    render(
      inspector(
        detail({
          healthReasons: [
            {
              text: "something the host could not classify",
              reasonCode: null,
              subjectKind: null,
              subjectLabel: null,
              owner: "evidence",
              sourceRecordId: "evidence-9",
              sourceField: "verification",
              sourceRevision: 1,
              asOfMillis: 1_699_000_000_000,
            },
          ],
        }),
      ),
    );

    expect(await screen.findByText(/something the host could not classify/)).toBeInTheDocument();
  });

  it("shows work only under the person who carries it, labelled as carried", async () => {
    // There is no Product-level work list. The Action appears inside the
    // person's entry, beneath the accountability heading, and nowhere else.
    render(inspector());
    await screen.findByRole("heading", { name: "Synthetic Product" });
    await userEvent.click(screen.getByRole("tab", { name: "人員" }));

    const person = screen.getByRole("heading", { name: /Synthetic Person/ }).closest("li");
    expect(person).not.toBeNull();
    expect(within(person as HTMLElement).getByText(/Action action-1/)).toBeInTheDocument();
    expect(
      within(person as HTMLElement).getByText("此人目前持有（經由問責，非此 Product 所有）"),
    ).toBeInTheDocument();
    // The same text does not exist outside the person's entry.
    expect(screen.getAllByText(/Action action-1/)).toHaveLength(1);
  });

  it("states how many other Products the person is accountable for", async () => {
    render(inspector());
    await screen.findByRole("heading", { name: "Synthetic Product" });
    await userEvent.click(screen.getByRole("tab", { name: "人員" }));
    expect(screen.getByText(/另負責 1 個 Product/)).toBeInTheDocument();
  });

  it("renders legal intents as text and offers no control to run them", async () => {
    render(inspector());
    await screen.findByRole("heading", { name: "Synthetic Product" });
    await userEvent.click(screen.getByRole("tab", { name: "人員" }));

    const item = screen.getByText(/Action action-1/).closest("li") as HTMLElement;
    // In words, as the Work Queue shows them: identifiers never reach a reader.
    expect(within(item).getByText(/開始進行、取消/)).toBeInTheDocument();
    expect(within(item).queryByText(/start_action/)).toBeNull();
    expect(within(item).queryByRole("button")).toBeNull();
  });

  it("presents an Initiative as reached through its Project", async () => {
    render(inspector());
    await screen.findByRole("heading", { name: "Synthetic Product" });
    const items = screen.getAllByRole("listitem");
    const initiative = items.find((item) => item.textContent.includes("Synthetic Initiative"));
    const project = items.find((item) => item.textContent.startsWith("Project"));
    // Named by the Project it was reached through, not its identifier.
    expect(initiative).toHaveTextContent("經由 Synthetic Project");
    expect(initiative).not.toHaveTextContent("project:project-1");
    expect(project).not.toHaveTextContent("經由");
  });

  it("keeps the Evidence classification and the link-time classification apart", async () => {
    render(inspector());
    await screen.findByRole("heading", { name: "Synthetic Product" });
    await userEvent.click(screen.getByRole("tab", { name: "Evidence" }));
    const panel = screen.getByRole("tabpanel", { name: "Evidence" });
    expect(within(panel).getByText(/evidence-1/)).toHaveTextContent(
      "Evidence 分級 Restricted；連結時分級 Internal",
    );
  });

  it("marks Evidence that carries no pinned fingerprint", async () => {
    render(inspector());
    await screen.findByRole("heading", { name: "Synthetic Product" });
    await userEvent.click(screen.getByRole("tab", { name: "Evidence" }));
    const panel = screen.getByRole("tabpanel", { name: "Evidence" });
    expect(within(panel).getByText(/evidence-1/)).toHaveTextContent("未釘選指紋");
  });

  it("refuses to render when the two Ledger reads disagree", async () => {
    render(inspector(detail({ state: "outOfSync", product: null })));
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(screen.queryByRole("tablist")).toBeNull();
  });

  it("shows no stale data when the load fails", async () => {
    render(
      <ProductInspector
        productId="product-1"
        load={() => Promise.reject(new Error("unavailable"))}
      />,
    );
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Synthetic Product" })).toBeNull();
  });

  it("says plainly when nothing needs attention", async () => {
    render(inspector(detail({ healthReasons: [] })));
    expect(await screen.findByText("目前沒有任何需要注意的狀況。")).toBeInTheDocument();
  });
});

function actionsSpy(overrides: Partial<EvidenceActions> = {}) {
  const calls: {
    command: string;
    evidenceId: string;
    expectedVersion: number;
    clientRequestId: string;
    expectedProductVersion?: number | undefined;
  }[] = [];
  const outcome = (changed: boolean, kind: string): EvidenceWriteOutcomeDto => ({
    changed,
    evidence: {
      id: "evidence-1",
      role: null,
      verification: { kind, atMillis: null, integrityDigest: null },
      pinned: kind !== "unverified",
      classification: "restricted",
      version: 4,
    },
    correlationId: "host-1",
  });
  const actions: EvidenceActions = {
    pinEvidenceFingerprint: (evidenceId, expectedVersion, clientRequestId) => {
      calls.push({ command: "pin", evidenceId, expectedVersion, clientRequestId });
      return Promise.resolve(outcome(true, "observed_unpinned"));
    },
    reobserveEvidenceVerification: (evidenceId, expectedVersion, clientRequestId) => {
      calls.push({ command: "reobserve", evidenceId, expectedVersion, clientRequestId });
      return Promise.resolve(outcome(false, "unverified"));
    },
    loadVaultStatus: () =>
      Promise.resolve<VaultStatusDto>({
        configured: true,
        available: true,
        reason: null,
        folderName: null,
        correlationId: "host-0",
      }),
    loadEvidenceReferences: () =>
      Promise.resolve<EvidenceReferencesDto>({
        ledgerRevision: 7,
        evidenceReferences: [
          {
            id: "evidence-1",
            role: null,
            verification: { kind: "unverified", atMillis: null, integrityDigest: null },
            pinned: false,
            classification: "restricted",
            version: 3,
          },
          {
            id: "evidence-9",
            role: null,
            verification: { kind: "verified", atMillis: 1, integrityDigest: null },
            pinned: true,
            classification: "internal",
            version: 5,
          },
        ],
        correlationId: "host-0",
      }),
    linkEvidenceToProduct: (
      productId,
      evidenceId,
      expectedVersion,
      clientRequestId,
      expectedProductVersion,
    ) => {
      calls.push({
        command: `link:${productId}`,
        evidenceId,
        expectedVersion,
        clientRequestId,
        expectedProductVersion,
      });
      return Promise.resolve({
        evidenceId,
        productId,
        classificationAtLink: "restricted",
        correlationId: "host-2",
      });
    },
    ...overrides,
  };
  return { actions, calls };
}

function withActions(actions: EvidenceActions, dto: ProductDetailDto = detail()) {
  return (
    <ProductInspector productId="product-1" load={() => Promise.resolve(dto)} actions={actions} />
  );
}

describe("ProductInspector Evidence writes", () => {
  /** Without an adapter the tab is exactly what it was: read-only. */
  it("offers no Evidence writes when no actions are supplied", async () => {
    render(inspector());
    await userEvent.click(await screen.findByRole("tab", { name: "Evidence" }));
    expect(screen.queryByRole("button", { name: "釘選指紋" })).toBeNull();
    expect(screen.queryByRole("button", { name: "重新觀察" })).toBeNull();
  });

  /**
   * A pin is the reference's identity and is never rewritten, so the action
   * exists only while there is no pin. Re-observation always applies.
   */
  it("offers the pin only while the reference is unpinned", async () => {
    const { actions } = actionsSpy();
    const pinned = detail({
      evidence: [
        {
          id: "evidence-1",
          role: null,
          verification: "verified",
          verifiedAtMillis: 5_000,
          pinned: true,
          classification: "restricted",
          classificationAtLink: "internal",
          linkedAtMillis: 6_000,
          revision: 3,
        },
      ],
    });
    render(withActions(actions, pinned));
    await userEvent.click(await screen.findByRole("tab", { name: "Evidence" }));
    expect(screen.queryByRole("button", { name: "釘選指紋" })).toBeNull();
    expect(screen.getByRole("button", { name: "重新觀察" })).toBeTruthy();
  });

  /**
   * The pin confirmation has to say the thing that cannot be undone, and it
   * must name the Evidence by id: this surface never receives a Vault path,
   * so it cannot name a file even if it wanted to.
   */
  it("states that a pin is permanent before running it, and names no path", async () => {
    const { actions, calls } = actionsSpy();
    render(withActions(actions));
    await userEvent.click(await screen.findByRole("tab", { name: "Evidence" }));
    await userEvent.click(screen.getByRole("button", { name: "釘選指紋" }));

    const confirmation = screen.getByRole("group");
    expect(within(confirmation).getByText(/釘選是永久的/)).toBeTruthy();
    expect(within(confirmation).getByText(/evidence-1/)).toBeTruthy();
    expect(confirmation.textContent).not.toContain("/");
    // Nothing ran on opening the confirmation.
    expect(calls).toHaveLength(0);

    await userEvent.click(screen.getByRole("button", { name: "確認釘選" }));
    expect(calls).toHaveLength(1);
    expect(calls[0]?.command).toBe("pin");
    expect(calls[0]?.evidenceId).toBe("evidence-1");
    // The version this view read (the fixture's entry is at revision 3).
    expect(calls[0]?.expectedVersion).toBe(3);
  });

  /**
   * `changed: false` is a real outcome. Reporting it as a write would claim
   * a Ledger effect that never happened.
   */
  it("says plainly when a re-observation wrote nothing", async () => {
    const { actions } = actionsSpy();
    render(withActions(actions));
    await userEvent.click(await screen.findByRole("tab", { name: "Evidence" }));
    await userEvent.click(screen.getByRole("button", { name: "重新觀察" }));
    await userEvent.click(screen.getByRole("button", { name: "確認重新觀察" }));

    expect(await screen.findByText(/Ledger 未被寫入/)).toBeTruthy();
  });

  /** A settled result must not hold the other controls shut. */
  it("frees the other Evidence actions once a result is closed", async () => {
    const { actions } = actionsSpy();
    render(withActions(actions));
    await userEvent.click(await screen.findByRole("tab", { name: "Evidence" }));
    await userEvent.click(screen.getByRole("button", { name: "重新觀察" }));
    await userEvent.click(screen.getByRole("button", { name: "確認重新觀察" }));
    await screen.findByText(/Ledger 未被寫入/);
    expect(screen.getByRole("button", { name: "連結 Evidence 到此 Product" })).toBeDisabled();

    await userEvent.click(screen.getByRole("button", { name: "關閉" }));
    expect(screen.getByRole("button", { name: "連結 Evidence 到此 Product" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "釘選指紋" })).toBeEnabled();
  });

  /**
   * The page around the inspector shows the same Product's measures, so a
   * write that landed must reach it too; one that wrote nothing must not
   * cause a re-read that implies otherwise.
   */
  it("tells the page to re-read only when a write changed the Ledger", async () => {
    const { actions } = actionsSpy();
    const changed = vi.fn();
    const view = () => (
      <ProductInspector
        productId="product-1"
        load={() => Promise.resolve(detail())}
        actions={actions}
        onLedgerChanged={changed}
      />
    );
    const first = render(view());
    await userEvent.click(await screen.findByRole("tab", { name: "Evidence" }));
    await userEvent.click(screen.getByRole("button", { name: "重新觀察" }));
    await userEvent.click(screen.getByRole("button", { name: "確認重新觀察" }));
    await screen.findByText(/Ledger 未被寫入/);
    expect(changed).not.toHaveBeenCalled();
    first.unmount();

    render(view());
    await userEvent.click(await screen.findByRole("tab", { name: "Evidence" }));
    await userEvent.click(screen.getByRole("button", { name: "釘選指紋" }));
    await userEvent.click(screen.getByRole("button", { name: "確認釘選" }));
    await screen.findByText(/已寫入：驗證狀態現在是/);
    expect(changed).toHaveBeenCalledTimes(1);
  });

  /**
   * A retry is a retry of the same command, so it carries the same request
   * id -- otherwise the host would treat it as a second write.
   */
  it("reuses the same client request id when a failed write is retried", async () => {
    const calls: string[] = [];
    let attempt = 0;
    const actions: EvidenceActions = {
      pinEvidenceFingerprint: (_evidenceId, _expectedVersion, clientRequestId) => {
        calls.push(clientRequestId);
        attempt += 1;
        if (attempt === 1) {
          // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- the host rejects with its safe envelope, not an Error.
          return Promise.reject({
            errorCode: "VAULT_ROOT_UNAVAILABLE",
            messageKey: "desktop.vault_root_unavailable",
            messageParams: [],
            correlationId: "host-9",
            retryable: true,
            extensions: [],
          });
        }
        return Promise.resolve({
          changed: true,
          evidence: {
            id: "evidence-1",
            role: null,
            verification: { kind: "observed_unpinned", atMillis: 1, integrityDigest: null },
            pinned: true,
            classification: "restricted",
            version: 4,
          },
          correlationId: "host-10",
        });
      },
      reobserveEvidenceVerification: () => {
        throw new Error("not used");
      },
      loadVaultStatus: () =>
        Promise.resolve({
          configured: true,
          available: true,
          reason: null,
          folderName: null,
          correlationId: "host-0",
        }),
      loadEvidenceReferences: () => {
        throw new Error("not used");
      },
      linkEvidenceToProduct: () => {
        throw new Error("not used");
      },
    };
    render(withActions(actions));
    await userEvent.click(await screen.findByRole("tab", { name: "Evidence" }));
    await userEvent.click(screen.getByRole("button", { name: "釘選指紋" }));
    await userEvent.click(screen.getByRole("button", { name: "確認釘選" }));

    await userEvent.click(await screen.findByRole("button", { name: "重試" }));
    expect(calls).toHaveLength(2);
    expect(calls[0]).toBe(calls[1]);
  });
});

describe("ProductInspector Degraded Mode and Product link", () => {
  /**
   * The visible Degraded Mode: the filesystem actions are shown
   * disabled with the reason, and the Ledger-only link is untouched.
   */
  it("disables pin and re-observe with a reason while the Vault is unavailable, and keeps link", async () => {
    const { actions } = actionsSpy({
      loadVaultStatus: () =>
        Promise.resolve({
          configured: true,
          available: false,
          reason: "invalidRoot",
          folderName: null,
          correlationId: "host-0",
        }),
    });
    render(withActions(actions));
    await userEvent.click(await screen.findByRole("tab", { name: "Evidence" }));

    expect(await screen.findByText(/Product Vault 目前無法讀取/)).toBeTruthy();
    expect(screen.getByRole<HTMLButtonElement>("button", { name: "釘選指紋" }).disabled).toBe(true);
    expect(screen.getByRole<HTMLButtonElement>("button", { name: "重新觀察" }).disabled).toBe(true);
    expect(
      screen.getByRole<HTMLButtonElement>("button", { name: "連結 Evidence 到此 Product" })
        .disabled,
    ).toBe(false);
  });

  /**
   * Candidates exclude what is already linked here; the target is the
   * inspected Product; the version sent is the candidate's own.
   */
  it("offers only unlinked Evidence and links it to the inspected Product at its version", async () => {
    const { actions, calls } = actionsSpy();
    render(withActions(actions));
    await userEvent.click(await screen.findByRole("tab", { name: "Evidence" }));
    await userEvent.click(screen.getByRole("button", { name: "連結 Evidence 到此 Product" }));

    const select = await screen.findByRole<HTMLSelectElement>("combobox");
    const offered = Array.from(select.options).map((option) => option.value);
    expect(offered).toEqual(["evidence-9"]);
    // Each option is worded, not the host's identifiers.
    expect(select.options[0]?.textContent).toBe("evidence-9（已驗證、Internal、版本 5）");

    await userEvent.click(screen.getByRole("button", { name: "確認連結" }));
    expect(calls).toHaveLength(1);
    expect(calls[0]?.command).toBe("link:product-1");
    expect(calls[0]?.evidenceId).toBe("evidence-9");
    expect(calls[0]?.expectedVersion).toBe(5);
    expect(await screen.findByText(/已連結 evidence-9/)).toBeTruthy();
  });
});

describe("ProductInspector Evidence from a file (item ⑦-3)", () => {
  const fileActions = () => ({
    chooseEvidenceFile: vi.fn(() =>
      Promise.resolve({
        chosen: true,
        token: "token-1",
        fileName: "launch-review.pdf",
        observedAtMillis: 1_700_000_000_000,
        existing: null,
        sameContent: [],
      }),
    ),
    createEvidenceFromFile: vi.fn(() =>
      Promise.resolve({
        outcome: "created" as const,
        evidence: { evidenceId: "evidence-7", version: 1 },
        observedAtMillis: 1_700_000_000_000,
      }),
    ),
    discardEvidenceFileChoice: vi.fn(() => Promise.resolve()),
  });

  it("disables the file action while the Vault is unavailable, like pin and re-observe", async () => {
    const { actions } = actionsSpy({
      evidenceFile: fileActions(),
      loadVaultStatus: () =>
        Promise.resolve({
          configured: false,
          available: false,
          reason: "notConfigured",
          folderName: null,
          correlationId: "host-0",
        }),
    });
    render(withActions(actions));
    await userEvent.click(await screen.findByRole("tab", { name: "Evidence" }));
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "從檔案新增 Evidence…" })).toBeDisabled();
    });
  });

  it("creates from a file, links it to the inspected Product, and reads the Product again", async () => {
    const evidenceFile = fileActions();
    const { actions, calls } = actionsSpy({ evidenceFile });
    const load = vi.fn(() => Promise.resolve(detail()));
    const onLedgerChanged = vi.fn();
    render(
      <ProductInspector
        productId="product-1"
        load={load}
        actions={actions}
        onLedgerChanged={onLedgerChanged}
      />,
    );
    await userEvent.click(await screen.findByRole("tab", { name: "Evidence" }));
    const open = screen.getByRole("button", { name: "從檔案新增 Evidence…" });
    await waitFor(() => {
      expect(open).toBeEnabled();
    });
    await userEvent.click(open);
    await userEvent.click(await screen.findByRole("button", { name: "選擇檔案…" }));
    await userEvent.click(await screen.findByRole("radio", { name: "Internal" }));
    await userEvent.click(screen.getByRole("button", { name: "建立並連結到這個 Product" }));
    expect(
      await screen.findByText("已建立 Evidence evidence-7，並連結到 Synthetic Product。"),
    ).toBeInTheDocument();
    expect(calls).toEqual([
      expect.objectContaining({
        command: "link:product-1",
        evidenceId: "evidence-7",
        expectedVersion: 1,
        // The Product version this view read (§4 Boundary).
        expectedProductVersion: 2,
      }),
    ]);
    const reads = load.mock.calls.length;
    await userEvent.click(screen.getByRole("button", { name: "完成" }));
    expect(load.mock.calls.length).toBeGreaterThan(reads);
    expect(onLedgerChanged).toHaveBeenCalled();
  });
});

describe("ProductInspector one write in flight (review follow-up)", () => {
  /**
   * The amendment promises one write at a time. That has to hold across
   * kinds: an entry mid-action withholds the link control, and a link past
   * idle withholds every entry action.
   */
  it("allows only one Evidence write in flight across kinds", async () => {
    const { actions } = actionsSpy();
    render(withActions(actions));
    await userEvent.click(await screen.findByRole("tab", { name: "Evidence" }));

    // An entry is mid-action (confirming): the link control is withheld.
    await userEvent.click(screen.getByRole("button", { name: "釘選指紋" }));
    expect(
      screen.getByRole<HTMLButtonElement>("button", { name: "連結 Evidence 到此 Product" })
        .disabled,
    ).toBe(true);
    await userEvent.click(screen.getByRole("button", { name: "取消" }));

    // The link control is past idle (choosing): entry actions are withheld.
    await userEvent.click(screen.getByRole("button", { name: "連結 Evidence 到此 Product" }));
    await screen.findByRole("combobox");
    expect(screen.getByRole<HTMLButtonElement>("button", { name: "釘選指紋" }).disabled).toBe(true);
    expect(screen.getByRole<HTMLButtonElement>("button", { name: "重新觀察" }).disabled).toBe(true);
  });
});

describe("ProductInspector record entry (slice 6B)", () => {
  function structured() {
    return detail({
      product: {
        id: "product-1",
        label: "Synthetic Product",
        classification: "internal",
        revision: 2,
        owner: "portfolio",
        asOfMillis: 1_700_000_000_000,
      },
      classificationForcedBy: null,
      structure: [
        {
          kind: "kpi_definition",
          id: "kpi-1",
          label: "Synthetic KPI",
          classification: "internal",
          revision: 1,
          via: null,
        },
      ],
    });
  }

  it("offers no entry actions without the entry actions", async () => {
    render(inspector(structured()));
    await screen.findByRole("heading", { name: "Synthetic Product" });
    expect(screen.queryByRole("button", { name: "新增 Roadmap…" })).toBeNull();
    expect(screen.queryByRole("button", { name: "編輯…" })).toBeNull();
  });

  it("creates a Roadmap from the Structure tab and links it to this Product at its version", async () => {
    const actions = entryTestActions();
    render(
      <ProductInspector
        productId="product-1"
        load={() => Promise.resolve(structured())}
        entryActions={actions}
      />,
    );
    await screen.findByRole("heading", { name: "Synthetic Product" });
    await userEvent.click(screen.getByRole("button", { name: "新增 Roadmap…" }));
    const dialog = await screen.findByRole("dialog", { name: "新增 Roadmap" });
    await userEvent.type(within(dialog).getByLabelText("名稱"), "R1");
    await userEvent.type(within(dialog).getByLabelText("說明"), "First roadmap.");
    await userEvent.click(within(dialog).getByLabelText("Internal"));
    await userEvent.click(within(dialog).getByRole("button", { name: "建立" }));

    await waitFor(() => {
      expect(actions.linkProductRoadmap).toHaveBeenCalledTimes(1);
    });
    const [createFields, createRequest] = (actions.createRoadmap as ReturnType<typeof vi.fn>).mock
      .calls[0] as [unknown, string];
    expect(createFields).toEqual({
      name: "R1",
      details: "First roadmap.",
      classification: "internal",
    });
    expect(actions.linkProductRoadmap).toHaveBeenCalledWith(
      "product-1",
      2,
      "roadmap-new",
      1,
      `${createRequest}-link`,
    );
    expect(await screen.findByRole("status")).toHaveTextContent("已建立 Roadmap roadmap-new。");
  });

  it("says when a created KPI exists but could not be linked", async () => {
    const actions = entryTestActions({
      linkProductKpi: vi.fn(() =>
        Promise.reject(
          Object.assign(new Error("host refused"), {
            errorCode: "DOMAIN_CONFLICT",
            messageKey: "ledger.version_stale",
            correlationId: "host-9",
            retryable: false,
            messageParams: [],
            extensions: [],
          }),
        ),
      ),
    });
    render(
      <ProductInspector
        productId="product-1"
        load={() => Promise.resolve(structured())}
        entryActions={actions}
      />,
    );
    await screen.findByRole("heading", { name: "Synthetic Product" });
    await userEvent.click(screen.getByRole("button", { name: "新增 KPI…" }));
    const dialog = await screen.findByRole("dialog", { name: "新增 KPI" });
    for (const [label, value] of [
      ["名稱", "K1"],
      ["定義", "What it counts."],
      ["負責人", "Owner"],
      ["目標", "10"],
      ["頻率", "monthly"],
      ["來源", "Counted."],
    ] as const) {
      fireEvent.change(within(dialog).getByLabelText(label), { target: { value } });
    }
    await userEvent.click(within(dialog).getByLabelText("Internal"));
    await userEvent.click(within(dialog).getByRole("button", { name: "建立" }));
    expect(await screen.findByRole("status")).toHaveTextContent(
      "已建立 KPI kpi-new，但無法連結到這個 Product。",
    );
    expect(screen.getByRole("status")).toHaveTextContent("連結既有的 KPI…");
  });

  it("records an observation against a KPI in the zone it was given", async () => {
    const actions = entryTestActions();
    render(
      <ProductInspector
        productId="product-1"
        load={() => Promise.resolve(structured())}
        entryActions={actions}
        timeZone="Asia/Taipei"
      />,
    );
    await screen.findByRole("heading", { name: "Synthetic Product" });
    await userEvent.click(screen.getByRole("button", { name: "記錄一筆觀測值…" }));
    const dialog = await screen.findByRole("dialog", { name: "為 Synthetic KPI 記錄一筆觀測值" });
    fireEvent.change(within(dialog).getByLabelText("數值"), { target: { value: "7" } });
    fireEvent.change(within(dialog).getByLabelText("觀測時間"), {
      target: { value: "2026-09-22T09:30" },
    });
    fireEvent.change(within(dialog).getByLabelText("來源"), { target: { value: "Counted." } });
    await userEvent.click(within(dialog).getByLabelText("與 KPI 相同"));
    await userEvent.click(within(dialog).getByRole("button", { name: "建立" }));
    await waitFor(() => {
      expect(actions.createKpiObservation).toHaveBeenCalledWith(
        "kpi-1",
        1,
        {
          value: "7",
          observedAtMillis: Date.UTC(2026, 8, 22, 1, 30),
          source: "Counted.",
          classification: null,
        },
        expect.stringMatching(/^o01-/),
      );
    });
  });

  it("edits the Product from the record the host holds, at that version", async () => {
    const actions = entryTestActions({}, [
      {
        kind: "product",
        id: "product-1",
        name: "Synthetic Product",
        details: "As held.",
        classification: "internal",
        version: 2,
      },
    ]);
    render(
      <ProductInspector
        productId="product-1"
        load={() => Promise.resolve(structured())}
        entryActions={actions}
      />,
    );
    await screen.findByRole("heading", { name: "Synthetic Product" });
    await userEvent.click(
      within(screen.getByRole("group", { name: "更動這個 Product" })).getByRole("button", {
        name: "編輯…",
      }),
    );
    const dialog = await screen.findByRole("dialog", { name: "編輯 Product" });
    expect(within(dialog).getByLabelText("說明")).toHaveValue("As held.");
    await userEvent.click(within(dialog).getByLabelText("Restricted"));
    await userEvent.click(within(dialog).getByRole("button", { name: "儲存" }));
    await waitFor(() => {
      expect(actions.updateProduct).toHaveBeenCalledWith(
        "product-1",
        2,
        { name: "Synthetic Product", details: "As held.", classification: "restricted" },
        expect.stringMatching(/^o01-/),
      );
    });
  });
});

describe("ProductInspector Delivery entry (slice 6C)", () => {
  function withProject() {
    return detail({
      product: {
        id: "product-1",
        label: "Synthetic Product",
        classification: "internal",
        revision: 2,
        owner: "portfolio",
        asOfMillis: 1_700_000_000_000,
      },
      classificationForcedBy: null,
      structure: [
        {
          kind: "project",
          id: "project-1",
          label: "Synthetic Project",
          classification: "internal",
          revision: 3,
          via: null,
        },
        {
          kind: "milestone",
          id: "milestone-1",
          label: "Synthetic Milestone",
          classification: "internal",
          revision: 1,
          via: null,
        },
        {
          kind: "initiative",
          id: "initiative-1",
          label: "Synthetic Initiative",
          classification: "internal",
          revision: 1,
          via: "project:project-1",
        },
      ],
    });
  }

  it("creates a Project from the Structure tab and links it to this Product", async () => {
    const actions = entryTestActions();
    render(
      <ProductInspector
        productId="product-1"
        load={() => Promise.resolve(withProject())}
        entryActions={actions}
        timeZone="Asia/Taipei"
      />,
    );
    await screen.findByRole("heading", { name: "Synthetic Product" });
    await userEvent.click(screen.getByRole("button", { name: "新增 Project…" }));
    const dialog = await screen.findByRole("dialog", { name: "新增 Project" });
    fireEvent.change(within(dialog).getByLabelText("名稱"), { target: { value: "P2" } });
    fireEvent.change(within(dialog).getByLabelText("開始"), {
      target: { value: "2026-10-01T09:00" },
    });
    // An end before the start is refused before any round trip.
    fireEvent.change(within(dialog).getByLabelText("結束"), {
      target: { value: "2026-09-01T09:00" },
    });
    await userEvent.click(within(dialog).getByLabelText("Internal"));
    expect(within(dialog).getByText("結束時間早於開始時間。")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "建立" })).toBeDisabled();
    fireEvent.change(within(dialog).getByLabelText("結束"), {
      target: { value: "2026-12-01T18:00" },
    });
    await userEvent.click(within(dialog).getByRole("button", { name: "建立" }));
    await waitFor(() => {
      expect(actions.linkProjectProduct).toHaveBeenCalledTimes(1);
    });
    const [fields, request] = (actions.createProject as ReturnType<typeof vi.fn>).mock.calls[0] as [
      unknown,
      string,
    ];
    expect(fields).toEqual({
      name: "P2",
      startAtMillis: Date.UTC(2026, 9, 1, 1, 0),
      endAtMillis: Date.UTC(2026, 11, 1, 10, 0),
      classification: "internal",
    });
    expect(actions.linkProjectProduct).toHaveBeenCalledWith(
      "project-new",
      1,
      "product-1",
      2,
      `${request}-link`,
    );
    expect(await screen.findByRole("status")).toHaveTextContent("已建立 Project project-new。");
  });

  it("enters a Milestone under its Project and an Initiative linked to it", async () => {
    const actions = entryTestActions();
    render(
      <ProductInspector
        productId="product-1"
        load={() => Promise.resolve(withProject())}
        entryActions={actions}
        timeZone="Asia/Taipei"
      />,
    );
    await screen.findByRole("heading", { name: "Synthetic Product" });
    const project = screen.getByRole("group", { name: "加到這個 Project" });
    await userEvent.click(within(project).getByRole("button", { name: "新增 Milestone…" }));
    const milestone = await screen.findByRole("dialog", {
      name: "在 Synthetic Project 下新增 Milestone",
    });
    fireEvent.change(within(milestone).getByLabelText("名稱"), { target: { value: "M2" } });
    fireEvent.change(within(milestone).getByLabelText("驗證標準"), {
      target: { value: "Shipped." },
    });
    fireEvent.change(within(milestone).getByLabelText("到期"), {
      target: { value: "2026-11-30T17:00" },
    });
    await userEvent.click(within(milestone).getByLabelText("Confidential"));
    await userEvent.click(within(milestone).getByRole("button", { name: "建立" }));
    await waitFor(() => {
      expect(actions.createMilestone).toHaveBeenCalledWith(
        "project-1",
        3,
        {
          name: "M2",
          verificationCriteria: "Shipped.",
          dueAtMillis: Date.UTC(2026, 10, 30, 9, 0),
          classification: "confidential",
        },
        expect.stringMatching(/^o01-/),
      );
    });

    await userEvent.click(
      within(screen.getByRole("group", { name: "加到這個 Project" })).getByRole("button", {
        name: "新增 Initiative…",
      }),
    );
    const initiative = await screen.findByRole("dialog", {
      name: "在 Synthetic Project 下新增 Initiative",
    });
    fireEvent.change(within(initiative).getByLabelText("名稱"), { target: { value: "I2" } });
    fireEvent.change(within(initiative).getByLabelText("預期成果"), {
      target: { value: "An outcome." },
    });
    await userEvent.click(within(initiative).getByLabelText("Internal"));
    await userEvent.click(within(initiative).getByRole("button", { name: "建立" }));
    await waitFor(() => {
      expect(actions.linkInitiativeProject).toHaveBeenCalledWith(
        "initiative-new",
        1,
        "project-1",
        3,
        expect.stringMatching(/^o01-.*-link$/),
      );
    });
  });

  it("links an existing Initiative not yet reached through this Project", async () => {
    const actions = entryTestActions({}, [
      {
        kind: "initiative",
        id: "initiative-1",
        name: "Synthetic Initiative",
        definedOutcome: "Reached.",
        classification: "internal",
        version: 1,
      },
      {
        kind: "initiative",
        id: "initiative-2",
        name: "Another Initiative",
        definedOutcome: "Not yet.",
        classification: "public",
        version: 4,
      },
    ]);
    render(
      <ProductInspector
        productId="product-1"
        load={() => Promise.resolve(withProject())}
        entryActions={actions}
      />,
    );
    await screen.findByRole("heading", { name: "Synthetic Product" });
    await userEvent.click(
      within(screen.getByRole("group", { name: "加到這個 Project" })).getByRole("button", {
        name: "連結既有的 Initiative…",
      }),
    );
    const dialog = await screen.findByRole("dialog", {
      name: "把 Initiative 連結到 Synthetic Project",
    });
    const options = within(dialog)
      .getAllByRole("option")
      .map((option) => option.textContent);
    expect(options).toEqual(["", "Another Initiative（Public，版本 4）"]);
    await userEvent.selectOptions(within(dialog).getByLabelText("要連結的紀錄"), "initiative-2");
    expect(within(dialog).getByText("這個連結會以 Internal 分級記錄。")).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "連結" }));
    await waitFor(() => {
      expect(actions.linkInitiativeProject).toHaveBeenCalledWith(
        "initiative-2",
        4,
        "project-1",
        3,
        expect.stringMatching(/^o01-/),
      );
    });
  });

  it("edits a Milestone from the record the host holds, in the given zone", async () => {
    const actions = entryTestActions({}, [
      {
        kind: "milestone",
        id: "milestone-1",
        projectId: "project-1",
        name: "Synthetic Milestone",
        verificationCriteria: "As held.",
        dueAtMillis: Date.UTC(2026, 10, 30, 9, 0),
        classification: "internal",
        version: 1,
      },
    ]);
    render(
      <ProductInspector
        productId="product-1"
        load={() => Promise.resolve(withProject())}
        entryActions={actions}
        timeZone="Asia/Taipei"
      />,
    );
    await screen.findByRole("heading", { name: "Synthetic Product" });
    const item = screen.getByText(/Synthetic Milestone/).closest("li");
    if (item === null) {
      throw new Error("the milestone entry is listed");
    }
    await userEvent.click(within(item).getByRole("button", { name: "編輯…" }));
    const dialog = await screen.findByRole("dialog", { name: "編輯 Milestone" });
    expect(within(dialog).getByLabelText("到期")).toHaveValue("2026-11-30T17:00");
    fireEvent.change(within(dialog).getByLabelText("名稱"), { target: { value: "M1, renamed" } });
    await userEvent.click(within(dialog).getByRole("button", { name: "儲存" }));
    await waitFor(() => {
      expect(actions.updateMilestone).toHaveBeenCalledWith(
        "milestone-1",
        1,
        {
          name: "M1, renamed",
          verificationCriteria: "As held.",
          dueAtMillis: Date.UTC(2026, 10, 30, 9, 0),
          classification: "internal",
        },
        expect.stringMatching(/^o01-/),
      );
    });
  });
});
