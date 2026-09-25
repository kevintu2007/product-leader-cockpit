import { describe, expect, it } from "vitest";

import { classificationChoices, isChosen } from "./classificationChoice";
import { isDirty } from "./dirtyState";
import { localToUtcMillis, utcMillisToLocal } from "./localDateTime";
import { TEXT_LIMITS, textProblem, utf8ByteLength, withinByteLimit } from "./utf8Bytes";

describe("record-entry primitives", () => {
  it("counts bytes the way the Ledger does", () => {
    expect(utf8ByteLength("abc")).toBe(3);
    expect(utf8ByteLength("風險")).toBe(6);
    expect(utf8ByteLength("é")).toBe(2);
    // ShortText is 160 bytes: 53 three-byte characters fit, 54 do not.
    expect(withinByteLimit("風".repeat(53), TEXT_LIMITS.short)).toBe(true);
    expect(withinByteLimit("風".repeat(54), TEXT_LIMITS.short)).toBe(false);
    expect(withinByteLimit("風".repeat(80), TEXT_LIMITS.title)).toBe(true);
    expect(withinByteLimit("風".repeat(81), TEXT_LIMITS.title)).toBe(false);
  });

  it("refuses what BoundedText::parse refuses, in the same order", () => {
    expect(textProblem("  ", TEXT_LIMITS.short)).toBe("empty");
    expect(textProblem("風".repeat(54), TEXT_LIMITS.short)).toBe("tooLong");
    expect(textProblem("two\nlines", TEXT_LIMITS.long)).toBe("control");
    expect(textProblem("tab\there", TEXT_LIMITS.long)).toBe("control");
    expect(textProblem("plain text", TEXT_LIMITS.short)).toBeNull();
  });

  it("offers no Unclassified for work records and starts with nothing chosen", () => {
    expect(classificationChoices("general")).toContain("unclassified");
    expect(classificationChoices("work")).not.toContain("unclassified");
    expect(classificationChoices("work")).toHaveLength(4);
    expect(isChosen(undefined)).toBe(false);
    expect(isChosen("internal")).toBe(true);
  });

  it("converts a field in the configured zone to the UTC instant and back", () => {
    // 2026-09-22 09:30 in Taipei is 01:30 UTC.
    expect(localToUtcMillis("2026-09-22T09:30", "Asia/Taipei")).toBe(Date.UTC(2026, 8, 22, 1, 30));
    // A date alone is that day's midnight in the zone.
    expect(localToUtcMillis("2026-09-22", "Asia/Taipei")).toBe(Date.UTC(2026, 8, 21, 16, 0));
    // The zone matters, not the browser's.
    expect(localToUtcMillis("2026-09-22T09:30", "UTC")).toBe(Date.UTC(2026, 8, 22, 9, 30));
    expect(utcMillisToLocal(Date.UTC(2026, 8, 22, 1, 30), "Asia/Taipei", true)).toBe(
      "2026-09-22T09:30",
    );
    expect(utcMillisToLocal(Date.UTC(2026, 8, 22, 1, 30), "Asia/Taipei", false)).toBe("2026-09-22");
    // A daylight-saving transition still round-trips.
    expect(localToUtcMillis("2026-03-29T03:30", "Europe/Berlin")).toBe(
      Date.UTC(2026, 2, 29, 1, 30),
    );
    // A time the autumn transition repeats is ambiguous: refused, not guessed.
    expect(localToUtcMillis("2026-10-25T02:30", "Europe/Berlin")).toBeUndefined();
    // A time the spring transition skips does not exist: refused.
    expect(localToUtcMillis("2026-03-29T02:30", "Europe/Berlin")).toBeUndefined();
    expect(localToUtcMillis("2026-02-30", "Asia/Taipei")).toBeUndefined();
    expect(localToUtcMillis("not a date", "Asia/Taipei")).toBeUndefined();
  });

  it("is dirty only when a field differs as the person sees it", () => {
    const initial = { title: "", classification: undefined, due: null };
    expect(isDirty(initial, { title: "  ", classification: undefined, due: undefined })).toBe(
      false,
    );
    expect(isDirty(initial, { title: "x", classification: undefined, due: null })).toBe(true);
    expect(isDirty(initial, { title: "", classification: "internal", due: null })).toBe(true);
    expect(isDirty({ title: "a" }, { title: "a", extra: undefined })).toBe(false);
  });
});
