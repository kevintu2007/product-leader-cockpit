import { useCallback, useEffect, useRef, useState } from "react";

/** Reads how many Work Queue items are flagged. */
export type LoadFlaggedCount = () => Promise<number>;

/**
 * How many Work Queue items something has flagged, for the rail badge. Read
 * at launch and again whenever `refresh` is called (the app calls it after
 * every Work Queue read). A failed read shows no badge rather than a number
 * it could not back.
 */
export function useFlaggedWorkCount(
  load: LoadFlaggedCount,
): readonly [number | undefined, () => void] {
  const [count, setCount] = useState<number | undefined>(undefined);
  // Only the latest read may set the badge: an earlier read that settles
  // later describes an older Ledger.
  const latest = useRef(0);
  // Stable while `load` is, because the Work Queue calls it after every read
  // and keeps it in its own dependencies.
  const refresh = useCallback(() => {
    latest.current += 1;
    const sequence = latest.current;
    load().then(
      (total) => {
        if (sequence === latest.current) {
          setCount(total);
        }
      },
      () => {
        if (sequence === latest.current) {
          setCount(undefined);
        }
      },
    );
  }, [load]);
  useEffect(() => {
    refresh();
  }, [refresh]);
  return [count, refresh] as const;
}
