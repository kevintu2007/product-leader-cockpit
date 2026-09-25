import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { useFlaggedWorkCount } from "./useFlaggedWorkCount";

interface Pending {
  readonly resolve: (total: number) => void;
  readonly reject: (reason: unknown) => void;
}

/** A loader whose every read stays open until the test settles it. */
function manualLoader() {
  const reads: Pending[] = [];
  const load = () =>
    new Promise<number>((resolve, reject) => {
      reads.push({ resolve, reject });
    });
  return { load, reads };
}

describe("useFlaggedWorkCount", () => {
  it("keeps the latest read when an earlier one settles after it", async () => {
    const { load, reads } = manualLoader();
    const { result } = renderHook(() => useFlaggedWorkCount(load));
    expect(reads).toHaveLength(1);

    act(() => {
      result.current[1]();
    });
    expect(reads).toHaveLength(2);

    await act(async () => {
      reads[1]?.resolve(4);
      await Promise.resolve();
    });
    expect(result.current[0]).toBe(4);

    // The launch read describes the Ledger before the write; it must not win.
    await act(async () => {
      reads[0]?.resolve(5);
      await Promise.resolve();
    });
    expect(result.current[0]).toBe(4);
  });

  it("shows no count when the latest read fails", async () => {
    const { load, reads } = manualLoader();
    const { result } = renderHook(() => useFlaggedWorkCount(load));
    await act(async () => {
      reads[0]?.resolve(3);
      await Promise.resolve();
    });
    expect(result.current[0]).toBe(3);

    act(() => {
      result.current[1]();
    });
    await act(async () => {
      reads[1]?.reject(new Error("unavailable"));
      await Promise.resolve();
    });
    expect(result.current[0]).toBeUndefined();
  });
});
