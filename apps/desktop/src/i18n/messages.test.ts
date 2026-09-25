import { describe, expect, it } from "vitest";

import { translatorFor } from "./catalogs";
import { interpolate } from "./messages";

describe("messages", () => {
  it("fills named slots and leaves a slot with no value visible", () => {
    expect(interpolate("en", "{a} and {b}", { a: "one" })).toBe("one and {b}");
  });

  it("never fills a slot from an inherited property", () => {
    expect(interpolate("en", "{toString} {constructor}", {})).toBe("{toString} {constructor}");
  });

  it("writes numbers the way the language writes them", () => {
    expect(interpolate("en", "{n}", { n: 12345 })).toBe("12,345");
    expect(interpolate("es", "{n}", { n: 12345 })).toBe("12.345");
  });

  it("chooses the plural form the language's rules pick", () => {
    const en = translatorFor("en");
    expect(en.plural("workQueue.caption", 1, { from: 1, to: 1 })).toBe(
      "Ordered by the approved ranking policy. Showing 1–1 of 1 item.",
    );
    expect(en.plural("workQueue.caption", 17, { from: 1, to: 17 })).toBe(
      "Ordered by the approved ranking policy. Showing 1–17 of 17 items.",
    );
  });

  it("uses the one form Chinese has for every count", () => {
    const zh = translatorFor("zh-TW");
    expect(zh.plural("workQueue.caption", 1, { from: 1, to: 1 })).toBe(
      "依核可的排序政策排列。顯示第 1–1 筆，共 1 筆。",
    );
    expect(zh.plural("workQueue.caption", 17, { from: 1, to: 17 })).toBe(
      "依核可的排序政策排列。顯示第 1–17 筆，共 17 筆。",
    );
  });

  it("refuses, at compile time, a key that does not exist or a missing parameter", () => {
    const en = translatorFor("en");
    // Each call below must fail `tsc`; if one ever compiles, the unused
    // directive fails the typecheck instead. Never called: this is a type
    // test, and the calls are wrong on purpose.
    const typeChecksOnly = (): void => {
      // @ts-expect-error -- no such key
      en("workQueue.no_such_key");
      // @ts-expect-error -- `from`, `to` and `count` are required
      en("workQueue.caption.other");
      // @ts-expect-error -- a parameter the message does not have
      en("common.yes", { extra: 1 });
      // @ts-expect-error -- a plural message's own parameters are required too
      en.plural("workQueue.caption", 1);
      // @ts-expect-error -- and `count` is given by `plural`, not by the caller
      en.plural("workQueue.caption", 1, { from: 1, to: 1, count: 1 });
    };
    expect(typeof typeChecksOnly).toBe("function");
    expect(en("common.yes")).toBe("yes");
  });

  it("finds a key known only at run time, or says it has none", () => {
    const en = translatorFor("en");
    expect(en.lookup("safeError.ledger.open.busy", {})).toBe("The Product Ledger is busy.");
    expect(en.lookup("safeError.some.future_key", {})).toBeNull();
    expect(en.lookup("toString", {})).toBeNull();
  });
});
