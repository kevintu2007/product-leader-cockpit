import { useEffect, useRef, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from "react";

export interface FocusTrapDialogProps {
  readonly titleId: string;
  readonly descriptionId?: string;
  /** Called when the user presses Escape. DG3 Accessibility and
   * Interaction Contract: "Escape performs the documented safe close/
   * reject behavior". O03 (H2a) closes without deciding, as the DG1 brief
   * says Escape should; O04 (H2b) rejects. */
  readonly onEscape: () => void;
  readonly className?: string;
  readonly children: ReactNode;
}

/**
 * Shared focus-trap dialog shell for O03/O04.
 * "Dialogs trap focus, Escape performs the documented safe close/reject
 * behavior, and focus returns to the opener" (DG3 Accessibility and
 * Interaction Contract). On mount, moves focus to the first focusable
 * element inside the dialog and remembers whatever had focus beforehand;
 * on unmount, restores focus there. Tab/Shift+Tab wrap at the dialog's own
 * first/last focusable elements rather than escaping to the page behind it.
 */
export function FocusTrapDialog({
  titleId,
  descriptionId,
  onEscape,
  className,
  children,
}: FocusTrapDialogProps) {
  const dialogRef = useRef<HTMLDivElement>(null);
  const openerRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    openerRef.current =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const focusable = getFocusable(dialogRef.current);
    focusable[0]?.focus();
    return () => {
      openerRef.current?.focus();
    };
  }, []);

  function handleKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    if (event.key !== "Escape" && event.key !== "Tab") {
      return;
    }
    // A dialog opened inside another (the passphrase dialog inside the Vault
    // sheet) owns these keys alone: Escape closes only the innermost dialog,
    // and Tab wraps at its edges, not the outer one's.
    event.stopPropagation();
    if (event.key === "Escape") {
      event.preventDefault();
      onEscape();
      return;
    }
    const focusable = getFocusable(dialogRef.current);
    if (focusable.length === 0) {
      return;
    }
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (!first || !last) {
      return;
    }
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  return (
    // eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions -- role="dialog" containers legitimately own Escape/Tab-trap keydown handling; this is the standard accessible-dialog pattern, not an unrelated interaction bolted onto a static element.
    <div
      ref={dialogRef}
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      aria-describedby={descriptionId}
      className={className}
      onKeyDown={handleKeyDown}
    >
      {children}
    </div>
  );
}

function getFocusable(container: HTMLElement | null): HTMLElement[] {
  if (!container) {
    return [];
  }
  return Array.from(
    container.querySelectorAll<HTMLElement>(
      'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
    ),
  );
}
