import { useState } from "react";

import { isBackupRefusal, useBackupShortcut } from "../backup/BackupShortcut";
import { useT } from "../i18n/useT";

export interface SafeErrorAction {
  readonly label: string;
  readonly onSelect: () => void;
}

export interface SafeErrorDetailProps {
  /** Already-localized safe message text. Resolving `errorCode`/
   * `messageKey`/`messageParams` into this string is the caller's job --
   * this shared component only renders the safe envelope consistently. */
  readonly message: string;
  readonly correlationId: string;
  readonly retryable: boolean;
  readonly onRetry?: () => void;
  /** The envelope's error class. A write refused because a backup is due
   * or running adds "Open Backups" (DG3 backup-setup amendment §5). */
  readonly errorCode?: string | undefined;
  /** Legal next actions or a recovery hint (e.g. "檢查網路連線後重試"). */
  readonly nextActions?: readonly SafeErrorAction[];
}

/**
 * O05 Safe error detail: "Localized safe message,
 * Correlation ID, retryability, legal next actions, protected local detail
 * reference only" (DG3 Overlay and Contextual Surface Contract).
 *
 * Never receives or renders a raw message key, stack trace, filesystem
 * path, provider payload, or `privateDetailRef` contents -- the caller
 * supplies only the safe, already-localized fields DG3's Error Contract
 * allows onto the surface. `role="alert"` announces the error immediately
 * (implicit `aria-live="assertive"`). The Correlation ID copy button
 * copies only the ID itself, matching "Correlation ID is copyable; copied
 * content remains public-safe and excludes privateDetailRef contents."
 */
export function SafeErrorDetail({
  message,
  correlationId,
  retryable,
  onRetry,
  nextActions: givenActions = [],
  errorCode,
}: SafeErrorDetailProps) {
  const t = useT();
  const openBackups = useBackupShortcut();
  const nextActions =
    openBackups !== null && isBackupRefusal(errorCode)
      ? [...givenActions, { label: t("backup.openBackups"), onSelect: openBackups }]
      : givenActions;
  const [copied, setCopied] = useState(false);

  async function handleCopy() {
    try {
      await navigator.clipboard.writeText(correlationId);
      setCopied(true);
    } catch {
      setCopied(false);
    }
  }

  return (
    <div role="alert" className="pmc-safe-error">
      <p className="pmc-safe-error-message">{message}</p>
      <div className="pmc-safe-error-correlation">
        <span className="pmc-safe-error-correlation-label">{t("errorDetail.correlationId")}</span>
        <span className="pmc-safe-error-correlation-value">{correlationId}</span>
        <button type="button" className="pmc-safe-error-copy" onClick={() => void handleCopy()}>
          {copied ? t("errorDetail.copied") : t("errorDetail.copy")}
        </button>
      </div>
      {retryable && onRetry ? (
        <button type="button" className="pmc-safe-error-retry" onClick={onRetry}>
          {t("errorDetail.retry")}
        </button>
      ) : null}
      {nextActions.length > 0 ? (
        <ul className="pmc-safe-error-actions">
          {nextActions.map((action) => (
            <li key={action.label}>
              <button type="button" onClick={action.onSelect}>
                {action.label}
              </button>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}
