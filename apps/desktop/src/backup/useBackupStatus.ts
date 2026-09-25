import { useCallback, useEffect, useRef, useState } from "react";

import type { BackupStatusDto } from "./backupIpc";

/** While the host is checking or backing up, look again soon. */
const BUSY_POLL_MILLIS = 2_000;
/** Otherwise often enough that a backup falling due mid-session shows. */
const IDLE_POLL_MILLIS = 60_000;

/**
 * The backup status the policy strip and Settings → Backups share. Reads
 * through the one reviewed H0 query at launch, on `refresh`, and on one
 * interval (faster while the host is busy). Only the newest read may set the
 * status, so overlapping reads never leave an older answer on screen; a read
 * that fails keeps the last known status rather than inventing one. The
 * fourth value says whether the newest read failed, so a screen can tell
 * "not read yet" from "cannot be read".
 */
export function useBackupStatus(
  load: () => Promise<BackupStatusDto>,
): [BackupStatusDto | undefined, () => void, (status: BackupStatusDto) => void, boolean] {
  const [status, setStatus] = useState<BackupStatusDto | undefined>(undefined);
  const [readFailed, setReadFailed] = useState(false);
  const latest = useRef(0);
  // Reads still waiting for the host. A poll tick is skipped while one is,
  // so a slow host never gets a growing queue of reads.
  const inFlight = useRef(0);

  const refresh = useCallback(() => {
    latest.current += 1;
    const sequence = latest.current;
    inFlight.current += 1;
    load()
      .then(
        (next) => {
          if (sequence === latest.current) {
            setStatus(next);
            setReadFailed(false);
          }
        },
        () => {
          // Keep the last known status.
          if (sequence === latest.current) {
            setReadFailed(true);
          }
        },
      )
      .finally(() => {
        inFlight.current -= 1;
      });
  }, [load]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const busy = status?.state === "checking" || status?.state === "backing_up";
  useEffect(() => {
    const poll = setInterval(
      () => {
        if (inFlight.current === 0) {
          refresh();
        }
      },
      busy ? BUSY_POLL_MILLIS : IDLE_POLL_MILLIS,
    );
    return () => {
      clearInterval(poll);
    };
  }, [refresh, busy]);

  return [status, refresh, setStatus, readFailed];
}
