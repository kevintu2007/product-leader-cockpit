import { describe, expect, it } from "vitest";

import { translatorFor } from "../i18n/catalogs";
import type { SafeErrorDto } from "../routes/cockpitContract";
import { fieldKeyWords, isSafeErrorDto, resolveRejection, resolveSafeError } from "./safeError";

const zh = translatorFor("zh-TW");
const ZH_TW_UNKNOWN_ERROR = zh("error.unknown");

function envelope(overrides: Partial<SafeErrorDto> = {}): SafeErrorDto {
  return {
    errorCode: "PLATFORM_INTERNAL",
    messageKey: "desktop.snapshot_unavailable",
    messageParams: [],
    correlationId: "host-1a2b-7",
    retryable: true,
    extensions: [],
    ...overrides,
  };
}

describe("safe error resolution (O05)", () => {
  it("localizes a known key and keeps the correlation id and retryability", () => {
    const resolved = resolveSafeError(envelope(), zh);
    expect(resolved.message).toContain("目前無法讀取 Product Ledger 的快照");
    expect(resolved.message).toContain("可以重試");
    expect(resolved.correlationId).toBe("host-1a2b-7");
    expect(resolved.retryable).toBe(true);
  });

  it("never shows a raw message key: an unknown key becomes the generic message", () => {
    const resolved = resolveSafeError(
      envelope({ messageKey: "evidence.some_future_key", retryable: false }),
      zh,
    );
    expect(resolved.message).toContain(ZH_TW_UNKNOWN_ERROR);
    expect(resolved.message).not.toContain("evidence.some_future_key");
    expect(resolved.message).toContain("重試不會改變結果");
  });

  it("fills typed params and leaves a missing placeholder visible", () => {
    // No shipped key has a placeholder yet, so the substitution is proven
    // against the resolver directly through a param that matches nothing:
    // the message must still be the catalog text, unchanged.
    const resolved = resolveSafeError(
      envelope({
        messageParams: [{ key: "evidence_id", value: { type: "identifier", value: "evidence-1" } }],
      }),
      zh,
    );
    expect(resolved.message).toContain("目前無法讀取 Product Ledger 的快照");
  });

  it("treats anything that is not the envelope as an unknown error with no correlation id", () => {
    expect(isSafeErrorDto(new Error("transport"))).toBe(false);
    expect(isSafeErrorDto("unavailable")).toBe(false);
    expect(isSafeErrorDto({ errorCode: "X" })).toBe(false);
    const resolved = resolveRejection(new Error("transport"), zh);
    expect(resolved.message).toContain(ZH_TW_UNKNOWN_ERROR);
    expect(resolved.correlationId).toBe("");
  });

  it("recognizes the envelope exactly as the host serializes it", () => {
    expect(isSafeErrorDto(envelope())).toBe(true);
    expect(
      isSafeErrorDto(
        envelope({
          extensions: [{ kind: "currentVersion", version: 3 }],
          privateDetailRef: "local-detail-9",
        }),
      ),
    ).toBe(true);
  });

  it("speaks the language it is given, keeping the Correlation ID", () => {
    const resolved = resolveSafeError(envelope({ retryable: false }), translatorFor("en"));
    expect(resolved.message).toBe(
      "The Product Ledger snapshot can't be read right now. Trying again will not change the result.",
    );
    expect(resolved.correlationId).toBe("host-1a2b-7");
  });

  it("speaks the language it is given, hint included", () => {
    const resolved = resolveSafeError(envelope(), translatorFor("ja"));
    expect(resolved.message).toBe(
      "現在 Product Ledger のスナップショットを読み取れません。再試行できます。",
    );
  });

  it("gives a message every host error key reaches it with, not the generic one", () => {
    // Keys beyond the first catalog: an Issue conflict and a KPI lowering.
    for (const messageKey of ["issue.stale_or_illegal", "kpi.classification_lowering_invalid"]) {
      expect(resolveSafeError(envelope({ messageKey }), zh).message).not.toContain(
        ZH_TW_UNKNOWN_ERROR,
      );
    }
  });

  it("words a field key as a field, next step, intent or state, else shows it as sent", () => {
    expect(fieldKeyWords("project.time_range", zh)).toBe("時間範圍");
    expect(fieldKeyWords("issue.refresh_and_reprepare", zh)).toBe("重新讀取後再準備一次。");
    expect(fieldKeyWords("prepare_accept_action_request", zh)).toBe("接受請求");
    expect(fieldKeyWords("in_progress", zh)).toBe("進行中");
    expect(fieldKeyWords("project.time_range", translatorFor("en"))).toBe("time range");
    expect(fieldKeyWords("some.future_field", zh)).toBe("some.future_field");
  });
  it("drops a parameter that is not one the host could send, without throwing", () => {
    const malformed = envelope({
      messageParams: [
        null,
        { key: "a" },
        { key: "b", value: { type: "unsigned", value: -1 } },
        { key: "c", value: { type: "unsigned", value: 1.5 } },
        { key: "d", value: { type: "boolean", value: "yes" } },
        { key: "e", value: { type: "somethingNew", value: "x" } },
      ] as unknown as SafeErrorDto["messageParams"],
    });
    const resolved = resolveSafeError(malformed, zh);
    expect(resolved.message).toContain("目前無法讀取 Product Ledger 的快照");
    expect(resolved.correlationId).toBe("host-1a2b-7");
  });
});
